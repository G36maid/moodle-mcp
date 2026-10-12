use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;

/// `core_webservice_get_site_info` — no capability required.
/// Returns site name, current user, Moodle release, and the list of WS
/// functions the token may call.
pub async fn run(client: &MoodleClient) -> Result<Value, MoodleError> {
    let v: Value = client
        .call("core_webservice_get_site_info", &json!({}))
        .await?;
    Ok(v)
}
