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
    // Product level first (closed set): MOODLE_LOG_LEVEL=error|warn|info|
    // debug|trace — trace is the only content-bearing level (response body
    // snippets land in the log — users of shared machines should not enable
    // it). RUST_LOG stays as the advanced escape hatch
    // (EnvFilter per-module syntax); invalid product values warn and fall
    // through to it, then to info.
    let raw_level = std::env::var("MOODLE_LOG_LEVEL")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty());
    let filter = match raw_level.as_deref() {
        Some(level @ ("error" | "warn" | "info" | "debug" | "trace")) => EnvFilter::new(level),
        Some(invalid) => {
            eprintln!(
                "moodle-mcp: invalid MOODLE_LOG_LEVEL '{invalid}' (error|warn|info|debug|trace) — falling back to RUST_LOG/info"
            );
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
        }
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
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

/// Pick the zone timestamps render in: MOODLE_TIMEZONE if set (Config has
/// validated it), else what `tools::site::detect_timezone` finds, else UTC.
async fn resolve_timezone(client: &MoodleClient, env_tz: Option<&str>, timeout_secs: u64) {
    if let Some(name) = env_tz {
        let tz = moodle_mcp::util::parse_timezone(name)
            .expect("MOODLE_TIMEZONE validated at config load");
        moodle_mcp::util::set_timezone(tz);
        tracing::info!(%name, "timestamp timezone set from MOODLE_TIMEZONE");
        return;
    }
    let budget = std::time::Duration::from_secs(timeout_secs);
    match moodle_mcp::tools::site::detect_timezone(client, budget).await {
        Some((tz, source)) => {
            moodle_mcp::util::set_timezone(tz);
            tracing::info!(timezone = %tz, source, "timestamp timezone detected from Moodle");
        }
        None => tracing::info!(
            "could not detect the Moodle timezone — rendering timestamps in UTC; \
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
    resolve_timezone(&client, cfg.timezone.as_deref(), cfg.timeout_secs).await;
    if cfg.read_only {
        tracing::info!("read-only mode: write, upload and raw-call tools are disabled");
    }
    if !cfg.enable_ws_function {
        tracing::info!(
            "web-service escape hatch (list_ws_functions, call_ws_function) is off — set MOODLE_ENABLE_WS_FUNCTION=true to enable it"
        );
    }
    let server = MoodleMcp::new_with_policy(
        client,
        cfg.read_only,
        cfg.enable_ws_function,
        cfg.disabled_tools.clone(),
    );

    let running = server.serve(stdio()).await?;
    tracing::info!("moodle-mcp serving on stdio");
    running.waiting().await?;
    tracing::info!("client disconnected, shutting down");
    Ok(())
}
