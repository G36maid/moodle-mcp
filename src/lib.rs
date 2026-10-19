//! moodle-mcp library crate — re-exported by the binary in `src/main.rs`.
//!
//! - [`client::MoodleClient`]: Moodle Web Services REST client
//! - [`config::Config`]: environment configuration
//! - [`error::MoodleError`]: typed Moodle API errors
//! - [`tools::MoodleMcp`]: the rmcp MCP server surface
//! - [`util`]: shaping helpers

pub mod auth;
pub mod client;
pub mod config;
pub mod error;
pub mod output;
pub mod sso;
pub mod tools;
pub mod util;
