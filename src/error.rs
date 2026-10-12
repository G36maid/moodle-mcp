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
    #[error("Unexpected response from Moodle: {0}")]
    InvalidBody(String),

    /// Client-side configuration error.
    #[error("Configuration error: {0}")]
    Config(String),
}

/// The exception payload Moodle returns instead of the expected data.
#[derive(Debug, serde::Deserialize)]
pub struct MoodleException {
    pub exception: String,
    pub errorcode: String,
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
            MoodleError::Api { exception, errorcode, message } => format!(
                "Moodle API error {errorcode}: {message} (exception: {exception})\n\n\
                 Hints: `invalidtoken` → check MOODLE_TOKEN; `accesscontrolexception`/`nopermission` → \
                 the function is not in the token's external service or you lack the capability; \
                 `sitemaintenance` → site is under maintenance."
            ),
            MoodleError::Http(e) => format!("HTTP request to Moodle failed: {e}"),
            MoodleError::Json(e) => format!(
                "Moodle returned a non-JSON response: {e}. Web services may be disabled on the site."
            ),
            MoodleError::InvalidBody(e) => format!(
                "Moodle returned an unexpected response: {e}. Web services may be disabled on the site."
            ),
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
