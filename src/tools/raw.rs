use serde_json::Value;

use crate::client::MoodleClient;
use crate::error::MoodleError;

/// Escape hatch: call any Moodle web service function by name with free-form
/// parameters (nested objects/arrays are flattened to PHP bracket notation).
///
/// `allowed` is the list of functions the token may call (from
/// `core_webservice_get_site_info`); the call is rejected client-side when the
/// function is not in it.
pub async fn call_ws_function(
    client: &MoodleClient,
    wsfunction: &str,
    params: &Value,
    allowed: &[String],
) -> Result<Value, MoodleError> {
    if !allowed.is_empty() && !allowed.iter().any(|f| f == wsfunction) {
        let sample: Vec<&str> = allowed.iter().take(15).map(String::as_str).collect();
        return Err(MoodleError::Config(format!(
            "function '{wsfunction}' is not enabled for this token. \
             {}/{} functions available, e.g.: {}. \
             Call site_info to see the full list.",
            allowed.len(),
            allowed.len(),
            sample.join(", ")
        )));
    }
    let v: Value = client.call_raw(wsfunction, params).await?;
    Ok(v)
}
