//! moodle-mcp binary — MCP stdio server by default, plus the `auth` CLI.
//!
//! - `moodle-mcp`                  → serve MCP over stdio (env: MOODLE_URL, MOODLE_TOKEN)
//! - `moodle-mcp auth`             → interactive: prompt for site, username, password (masked)
//! - `moodle-mcp auth --url ...`   → obtain a web service token (see auth.rs)

use std::sync::Arc;

use moodle_mcp::client::MoodleClient;
use moodle_mcp::config::Config;
use moodle_mcp::tools::MoodleMcp;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        // stdout is reserved for the MCP JSON-RPC stream — never write there.
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

fn load_config() -> Config {
    match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("moodle-mcp: configuration error: {e}");
            eprintln!(
                "required env: MOODLE_URL=https://<moodle-host> MOODLE_TOKEN=<webservice token>"
            );
            std::process::exit(2);
        }
    }
}

/// Resolve the timestamp-rendering timezone: an explicit MOODLE_TIMEZONE
/// wins (already validated by Config); otherwise the user's Moodle timezone
/// preference is consulted (its value may be '99' = site default, which no
/// web service exposes, or otherwise unparseable) and anything unknown
/// falls back to UTC with a log line.
/// core_user_get_user_preferences is available to the mobile service on
/// every supported version:
/// https://github.com/moodle/moodle/blob/v3.7.0/lib/db/services.php#L1725
/// https://github.com/moodle/moodle/blob/v4.5.4/lib/db/services.php#L1999
async fn resolve_timezone(client: &MoodleClient, env_tz: Option<&str>) {
    if let Some(name) = env_tz {
        let tz = moodle_mcp::util::parse_timezone(name)
            .expect("MOODLE_TIMEZONE validated at config load");
        moodle_mcp::util::set_timezone(tz);
        tracing::info!(%name, "timestamp timezone set from MOODLE_TIMEZONE");
        return;
    }
    let detected = client
        .call(
            "core_user_get_user_preferences",
            &serde_json::json!({ "name": "timezone" }),
        )
        .await
        .ok()
        .and_then(|v| moodle_mcp::util::timezone_from_preferences(&v));
    match detected {
        Some(tz) => {
            moodle_mcp::util::set_timezone(tz);
            tracing::info!(timezone = %tz, "timestamp timezone detected from the Moodle user preference");
        }
        None => tracing::info!(
            "timezone unknown (site default or not exposed) — rendering timestamps in UTC; \
             set MOODLE_TIMEZONE to override"
        ),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("auth") => return moodle_mcp::auth::run(&args.collect::<Vec<_>>()).await,
        Some("serve") | None => {}
        Some(other) => {
            eprintln!("moodle-mcp: unknown subcommand '{other}'");
            eprintln!("usage: moodle-mcp [serve] | moodle-mcp auth [--url <site> ...]");
            std::process::exit(2);
        }
    }

    init_tracing();
    let cfg = load_config();
    tracing::info!(
        "starting moodle-mcp v{} for {}",
        env!("CARGO_PKG_VERSION"),
        cfg.base_url
    );

    let client = Arc::new(MoodleClient::new(cfg.clone())?);
    resolve_timezone(&client, cfg.timezone.as_deref()).await;
    if cfg.read_only {
        tracing::info!("read-only mode: write, upload and raw-call tools are disabled");
    }
    let server = MoodleMcp::new_with_policy(client, cfg.read_only, cfg.disabled_tools.clone());

    let running = server.serve(stdio()).await?;
    tracing::info!("moodle-mcp serving on stdio");
    running.waiting().await?;
    tracing::info!("client disconnected, shutting down");
    Ok(())
}
