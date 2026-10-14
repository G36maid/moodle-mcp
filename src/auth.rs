//! `moodle-mcp auth` — obtain a web service token without touching curl.
//!
//! Bare `moodle-mcp auth` is interactive: the site URL, username, and
//! password are prompted for one at a time (the password is masked with `*`
//! as you type). Every prompt can be skipped by passing the value as a flag
//! (`--url`, `--username`, `--password`), so scripts work non-interactively
//! too.
//!
//! - **credentials** mode (default): POST `/login/token.php` with
//!   username/password (NTNU-verified).
//! - **`--open`**: opens the site's Security keys page (`/user/managetoken.php`)
//!   in the browser — the SSO-friendly path; the user copies the token by hand.
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
    let mut open_browser = false;
    let mut output_json = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--url" => url = it.next().cloned(),
            "--username" | "-u" => username = it.next().cloned(),
            "--password" | "-p" => password = it.next().cloned(),
            "--service" | "-s" => service = it.next().cloned(),
            "--open" => open_browser = true,
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
                    "usage: moodle-mcp auth [--url <site>] [--username U] [--password P] [--service S] [--open] [--output env|json]\nvalues not passed as flags are prompted for interactively"
                        .into(),
                );
            }
        }
    }

    let base_url = match url {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => prompt_url()?,
    };

    if open_browser {
        let page = format!("{base_url}/user/managetoken.php");
        open::that_detached(&page)?;
        println!("Opened {page}");
        println!("Log in, open 'Security keys', copy the token, then:");
        println!("  export MOODLE_URL='{base_url}' MOODLE_TOKEN='<paste>'");
        return Ok(());
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

    let result =
        crate::client::MoodleClient::fetch_token(&base_url, &username, &password, &service).await;

    match result {
        Ok(payload) => {
            let token = payload
                .get("token")
                .and_then(Value::as_str)
                .ok_or("response did not contain a token")?;
            if output_json {
                println!("{payload}");
            } else {
                println!("Token obtained ({service}).");
                println!();
                println!("export MOODLE_URL='{base_url}' MOODLE_TOKEN='{token}'");
                println!();
                println!("The token is a credential — treat it like a password.");
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("{e}");
            Err("token acquisition failed".into())
        }
    }
}

/// Prompt for the site URL until it looks like an http(s) URL.
fn prompt_url() -> Result<String, Box<dyn std::error::Error>> {
    Text::new("Moodle URL (e.g. https://moodle.example.com)")
        .with_validator(|s: &str| {
            let s = s.trim();
            if s.starts_with("http://") || s.starts_with("https://") {
                Ok(Validation::Valid)
            } else {
                Ok(Validation::Invalid(
                    "enter the full site URL, starting with https://".into(),
                ))
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
