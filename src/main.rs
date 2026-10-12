//! moodle-mcp binary — wires config + client + MCP server onto stdio.
//!
//! Environment:
//! - `MOODLE_URL`   — base URL, e.g. `https://moodle.example.com`
//! - `MOODLE_TOKEN` — web service token
//! - `MOODLE_TIMEOUT_SECS` — optional request timeout (default 60)
//! - `RUST_LOG`     — optional tracing filter (default `info`); logs go to
//!   stderr because stdout is the MCP stdio transport.

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
            eprintln!("required env: MOODLE_URL=https://<moodle-host> MOODLE_TOKEN=<webservice token>");
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let cfg = load_config();
    tracing::info!("starting moodle-mcp v{} for {}", env!("CARGO_PKG_VERSION"), cfg.base_url);

    let client = Arc::new(MoodleClient::new(cfg)?);
    let server = MoodleMcp::new(client);

    let running = server.serve(stdio()).await?;
    tracing::info!("moodle-mcp serving on stdio");
    running.waiting().await?;
    tracing::info!("client disconnected, shutting down");
    Ok(())
}
