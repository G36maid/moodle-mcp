/// Error returned by a Moodle web service call.
#[derive(Debug, thiserror::Error)]
pub enum MoodleError {
    /// Moodle returned an exception payload (HTTP status is often still 200).
    #[error("Moodle API error [{errorcode}]: {message} (exception: {exception})")]
    Api {
        exception: String,
        errorcode: String,
        message: String,
    },

    /// HTTP-level failure (network, DNS, TLS, timeout...).
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Response body was not valid JSON (e.g. HTML error page, web services disabled).
    #[error("Invalid JSON response from Moodle (are web services enabled?): {0}")]
    Json(#[from] serde_json::Error),

    /// Response body was valid JSON but not the expected shape.
    #[error("{0}")]
    InvalidBody(String),

    /// Response body was not JSON at all (e.g. HTML login page).
    #[error("Non-JSON response from Moodle (web services may be disabled): {0}")]
    NonJsonResponse(String),

    /// Client-side configuration error.
    #[error("Configuration error: {0}")]
    Config(String),
}

/// The exception payload Moodle returns instead of the expected data.
/// `errorcode` may be absent on PHP fatals (e.g. 3.7 returns only
/// `{exception, message}`).
#[derive(Debug, serde::Deserialize)]
pub struct MoodleException {
    pub exception: String,
    #[serde(default)]
    pub errorcode: String,
    #[serde(default)]
    pub message: String,
}

impl MoodleError {
    pub fn from_exception(e: MoodleException) -> Self {
        MoodleError::Api {
            exception: e.exception,
            errorcode: e.errorcode,
            message: e.message,
        }
    }

    /// Render a tool-level error message that the LLM can read and act on.
    pub fn to_tool_message(&self) -> String {
        match self {
            MoodleError::Api {
                exception,
                errorcode,
                message,
            } => {
                // PHP fatals arrive with an empty errorcode — fall back to the
                // exception class name so the code shown is never empty.
                let code = if errorcode.is_empty() {
                    exception.as_str()
                } else {
                    errorcode.as_str()
                };
                let hint = match code {
                    "invalidtoken" => "check MOODLE_TOKEN — it may have been reset or expired.".to_string(),
                    "nopermissions" | "accesscontrolexception" | "required_capability_exception" => "the function is not enabled for this token, or you lack the capability on this site/course.".to_string(),
                    "sitemaintenance" => "the site is under maintenance — try again later.".to_string(),
                    "invalidrecord" | "dml_missing_record_exception" => "the referenced record does not exist — double-check the id (a wrong-type id, e.g. a module id instead of an instance id, can also look like this).".to_string(),
                    "invalidparameter" | "invalid_parameter_exception" | "invalidresponse" => "check the parameter names, types and value formats against what the tool asked for.".to_string(),
                    "filenameexist" => "a file with this name already exists in that draft area — upload under a different filename, or start a fresh draft with a new item_id.".to_string(),
                    "fileoversized" => "the file exceeds the site's upload size limit — check the file or ask for a higher quota.".to_string(),
                    "toomanyrequests" | "ratelimit" => "rate limited — wait a moment and retry.".to_string(),
                    _ => String::new(),
                };
                let hint_block = if hint.is_empty() {
                    String::new()
                } else {
                    format!("\n\nHints: {hint}")
                };
                format!("Moodle API error {code}: {message} (exception: {exception}){hint_block}")
            }
            MoodleError::Http(e) => scrub_token(&format!("HTTP request to Moodle failed: {e}")),
            MoodleError::Json(e) => format!(
                "Moodle returned a non-JSON response: {e}. Web services may be disabled on the site."
            ),
            MoodleError::InvalidBody(e) => format!("Moodle returned an unexpected response: {e}"),
            MoodleError::NonJsonResponse(e) => {
                format!("Non-JSON response from Moodle (web services may be disabled): {e}")
            }
            MoodleError::Config(e) => format!("Configuration error: {e}"),
        }
    }
}

/// Map a [`MoodleError`] onto an MCP tool-level error result (`isError: true`),
/// so the model can see the message and self-correct. Protocol errors
/// (`Err(McpError)`) hide the message from the model and should be reserved
/// for routing/infrastructure failures.
pub fn moodle_to_tool_result(err: MoodleError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(err.to_tool_message())])
}

use rmcp::model::{CallToolResult, ContentBlock};

/// Replace `token=…`-style query values with `[REDACTED]` in an arbitrary
/// string. reqwest errors embed the full request URL — query string included
/// — and download URLs carry the injected web-service token, which must never
/// reach the model, tool output, or logs.
pub(crate) fn scrub_token(s: &str) -> String {
    const KEY: &str = "token=";
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find(KEY) {
        out.push_str(&rest[..idx + KEY.len()]);
        let after = &rest[idx + KEY.len()..];
        // Token values are URL-safe ([A-Za-z0-9_-]); redact until the first
        // character that cannot belong to one (delimiter, space, paren…).
        let end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(after.len());
        out.push_str("[REDACTED]");
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod scrub_tests {
    use super::*;

    #[test]
    fn scrubs_token_values() {
        let msg = "error sending request for url \
                  (https://moodle.example.com/webservice/pluginfile.php/1/f.pdf?forcedownload=1&token=AbC_123-x)";
        let out = scrub_token(msg);
        assert!(out.contains("token=[REDACTED]"), "{out}");
        assert!(!out.contains("AbC_123-x"), "{out}");
        assert!(out.contains("pluginfile.php/1/f.pdf"), "keep path: {out}");

        assert_eq!(
            scrub_token("https://x/?token=SECRET&next=1"),
            "https://x/?token=[REDACTED]&next=1"
        );
        assert_eq!(
            scrub_token("https://x/?token=TAIL"),
            "https://x/?token=[REDACTED]"
        );
        assert_eq!(scrub_token("plain error text"), "plain error text");
    }

    #[tokio::test]
    async fn http_tool_message_never_leaks_token() {
        let err = reqwest::Client::new()
            .get("https://127.0.0.1:1/pluginfile.php/1/f.pdf?token=SECRET123")
            .send()
            .await
            .expect_err("must fail");
        let msg = MoodleError::Http(err).to_tool_message();
        assert!(!msg.contains("SECRET123"), "token leaked: {msg}");
        assert!(msg.contains("127.0.0.1"), "keep host for diagnosis: {msg}");
    }
}
