//! `moodle-mcp token` — obtain a web service token without touching curl.
//!
//! Two modes:
//! - **credentials** (default): POST `/login/token.php` with username/password
//!   (NTNU-verified). The password is prompted hidden when not given as an
//!   argument, so it never lands in shell history.
//! - **`--open`**: opens the site's Security keys page (`/user/managetoken.php`)
//!   in the browser — the SSO-friendly path; the user copies the token by hand.
//!
//! Output defaults to a ready-to-source `export` line; `--output json` prints
//! raw JSON instead.

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
                output_json = matches!(it.next().map(String::as_str), Some("json"));
            }
            other => {
                eprintln!("moodle-mcp token: unknown argument '{other}'");
                return Err("usage: moodle-mcp token --url <site> [--username U] [--password P] [--service S] [--open] [--output env|json]".into());
            }
        }
    }

    let base_url = url.ok_or("missing --url (e.g. https://moodle3.ntnu.edu.tw)")?;
    let base_url = base_url.trim_end_matches('/').to_string();

    if open_browser {
        let page = format!("{base_url}/user/managetoken.php");
        open::that_detached(&page)?;
        println!("Opened {page}");
        println!("Log in, open 'Security keys', copy the token, then:");
        println!("  export MOODLE_URL='{base_url}' MOODLE_TOKEN='<paste>'");
        return Ok(());
    }

    let username = username.ok_or("missing --username (or use --open for the browser flow)")?;
    let password = match password {
        Some(p) => p,
        None => rpassword::prompt_password("Password: ")?,
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
