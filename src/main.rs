//! moodle-mcp binary — MCP stdio server by default, plus the `token` CLI.
//!
//! - `moodle-mcp`                    → serve MCP over stdio (env: MOODLE_URL, MOODLE_TOKEN)
//! - `moodle-mcp token --url ...`    → obtain a web service token (see token.rs)

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);

    // Subcommand dispatch. `token` is the only real subcommand; `serve` (and
    // nothing at all) runs the MCP server over stdio.
    match args.next().as_deref() {
        Some("token") => return moodle_mcp::token::run(&args.collect::<Vec<_>>()).await,
        Some("serve") | None => {}
        Some(other) => {
            eprintln!("moodle-mcp: unknown subcommand '{other}'");
            eprintln!("usage: moodle-mcp [serve] | moodle-mcp token --url <site> ...");
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

    let client = Arc::new(MoodleClient::new(cfg)?);
    let server = MoodleMcp::new(client);

    let running = server.serve(stdio()).await?;
    tracing::info!("moodle-mcp serving on stdio");
    running.waiting().await?;
    tracing::info!("client disconnected, shutting down");
    Ok(())
}
