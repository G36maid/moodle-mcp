//! `moodle-mcp auth` — obtain a web service token without touching curl.
//!
//! Bare `moodle-mcp auth` is interactive: the site URL, username, and
//! password are prompted for one at a time (the password is masked with `*`
//! as you type). Every prompt can be skipped by passing the value as a flag
//! (`--url`, `--username`, `--password`), so scripts work non-interactively
//! too.
//!
//! - **credentials** mode: POST `/login/token.php` with username/password.
//!   Schools with a maintained SSO driver ([`crate::sso`], matched by host)
//!   skip this and replay their own login chain instead — auth picks the
//!   path per host, before any request moves.
//!
//! Output defaults to a ready-to-source `export` line; `--output json` prints
//! raw JSON instead. Prompts render on stderr, so `--output json` on stdout
//! stays pipeable.

use inquire::validator::Validation;
use inquire::{Password, PasswordDisplayMode, Text};
use serde_json::Value;

pub async fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut url: Option<String> = None;
    let mut username: Option<String> = None;
    let mut password: Option<String> = None;
    let mut service: Option<String> = None;
    let mut allow_http = false;
    let mut output_json = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--url" => url = it.next().cloned(),
            "--username" | "-u" => username = it.next().cloned(),
            "--password" | "-p" => password = it.next().cloned(),
            "--service" | "-s" => service = it.next().cloned(),
            "--allow-http" => allow_http = true,
            "--output" | "-o" => {
                let val = it
                    .next()
                    .cloned()
                    .ok_or("missing value after --output (env|json)")?;
                match val.as_str() {
                    "env" => output_json = false,
                    "json" => output_json = true,
                    other => {
                        return Err(format!("unknown --output '{other}' (use env|json)").into());
                    }
                }
            }
            other => {
                eprintln!("moodle-mcp auth: unknown argument '{other}'");
                return Err(
                    "usage: moodle-mcp auth [--url <site>] [--username U] [--password P] [--service S] [--allow-http] [--output env|json]\nvalues not passed as flags are prompted for interactively"
                        .into(),
                );
            }
        }
    }

    let base_url = match url {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => prompt_url()?,
    };
    // One policy for both entry points (serve validates MOODLE_URL the same
    // way): schemeless defaults to https; plain http is rejected unless
    // loopback or explicitly allowed — the password is about to cross it.
    let mut base_url = match crate::config::normalize_site_url(&base_url, allow_http) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("moodle-mcp auth: {e}");
            return Err("invalid site URL".into());
        }
    };
    // Schools migrate hosts (moodle.example.edu -> moodle3.example.edu) with
    // a bare 303 to the new homepage; every request to the old origin would
    // land on the front page instead of the page asked for.
    if let Some(final_origin) = crate::client::resolve_site_redirect(&base_url).await {
        eprintln!("moodle-mcp auth: note: {base_url} redirects to {final_origin} — using it");
        base_url = final_origin;
    }

    let username = match username {
        Some(u) => u,
        None => prompt_username()?,
    };
    let password = match password {
        Some(p) => p,
        None => prompt_password()?,
    };

    let service = service.unwrap_or_else(|| "moodle_mobile_app".to_string());

    // Schools with a maintained driver (src/sso/) log in through their own
    // SSO chain; everyone else through credentials mode — invalidlogin
    // there points at the missing driver (one PR away).
    let token_payload = if crate::sso::supports_sso(&base_url) {
        match crate::sso::sso_login(&base_url, &service, &username, &password).await {
            Ok(outcome) => serde_json::json!({ "token": outcome.token }),
            Err(e) => {
                eprintln!("moodle-mcp auth: {e}");
                return Err("token acquisition failed".into());
            }
        }
    } else {
        match crate::client::MoodleClient::fetch_token(&base_url, &username, &password, &service)
            .await
        {
            Ok(payload) => payload,
            Err(e) => {
                eprintln!("{e}");
                if e.to_string().contains("no local Moodle password") {
                    eprintln!(
                        "moodle-mcp auth: no SSO driver is maintained for this school yet — \
                         ask your admin for a token, or contribute one"
                    );
                }
                return Err("token acquisition failed".into());
            }
        }
    };

    let token = token_payload
        .get("token")
        .and_then(Value::as_str)
        .ok_or("response did not contain a token")?;
    if output_json {
        println!("{token_payload}");
    } else {
        println!("Token obtained ({service}).");
        println!();
        // The line is meant for `source`/`eval` — a token from a hostile
        // server must not be able to break out of the quoting.
        println!(
            "export MOODLE_URL='{}' MOODLE_TOKEN='{}'",
            shell_single_quoted(&base_url),
            shell_single_quoted(token)
        );
        println!();
        println!("The token is a credential — treat it like a password.");
    }
    Ok(())
}

/// Make a value safe inside single quotes for POSIX shells: everything
/// between single quotes is literal except the quote itself, which closes,
/// emits an escaped quote, and reopens.
fn shell_single_quoted(value: &str) -> String {
    value.replace('\'', "'\\''")
}

/// Prompt for the site URL until it looks like an http(s) URL.
fn prompt_url() -> Result<String, Box<dyn std::error::Error>> {
    // The scheme/host policy lives in `normalize_site_url` (applied after the
    // prompt returns) — the validator only catches obvious typos early.
    Text::new("Moodle URL (e.g. https://moodle.example.com)")
        .with_validator(|s: &str| {
            let s = s.trim();
            if s.is_empty() || s.contains(char::is_whitespace) {
                Ok(Validation::Invalid(
                    "enter the site URL, e.g. https://moodle.example.com (a bare host defaults to https)"
                        .into(),
                ))
            } else {
                Ok(Validation::Valid)
            }
        })
        .prompt()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .map_err(prompt_error)
}

/// Prompt for the username (must not be blank).
fn prompt_username() -> Result<String, Box<dyn std::error::Error>> {
    Text::new("Username")
        .with_validator(|s: &str| {
            if s.trim().is_empty() {
                Ok(Validation::Invalid("username cannot be empty".into()))
            } else {
                Ok(Validation::Valid)
            }
        })
        .prompt()
        .map(|s| s.trim().to_string())
        .map_err(prompt_error)
}

/// Prompt for the password. Masked display: one `*` per typed character,
/// backspace erases one — the characters themselves never reach the screen.
fn prompt_password() -> Result<String, Box<dyn std::error::Error>> {
    Password::new("Password")
        .with_display_mode(PasswordDisplayMode::Masked)
        .without_confirmation()
        .with_validator(|s: &str| {
            if s.trim().is_empty() {
                Ok(Validation::Invalid("password cannot be empty".into()))
            } else {
                Ok(Validation::Valid)
            }
        })
        .prompt()
        .map_err(prompt_error)
}

/// Turn prompt failures (Ctrl-C/Esc, or stdin not a terminal) into plain
/// advice.
fn prompt_error(err: inquire::InquireError) -> Box<dyn std::error::Error> {
    if matches!(
        err,
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted
    ) {
        return "prompt cancelled".into();
    }
    format!(
        "could not prompt for credentials ({err}); \
         pass --url, --username and --password as flags instead"
    )
    .into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn quoting_keeps_hostile_values_inert() {
        // a token from a hostile server must not break out of the quoting
        // when the printed export line is sourced: a'\''b re-enters the
        // quoting, so everything between the outer quotes stays literal.
        let hostile = "a'; rm -rf ~; '";
        let quoted = format!("'{}'", super::shell_single_quoted(hostile));
        assert_eq!(
            quoted,
            concat!("'", "a", "'\\''", "; rm -rf ~; ", "'\\''", "'")
        );
        assert_eq!(
            super::shell_single_quoted("plain-token-123"),
            "plain-token-123"
        );
    }
}
