//! Users: single-profile lookup and (best-effort) search.

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{html_to_markdown, unix_to_iso};

/// `core_user_get_users_by_field` — look up a user by an exact key
/// (username / email / id / idnumber). Which fields come back (and whether
/// other students are visible at all) is the site's privacy policy.
pub async fn get_user_by_field(
    client: &MoodleClient,
    field: &str,
    value: &str,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "core_user_get_users_by_field",
            &json!({ "field": field, "values": [value] }),
        )
        .await?;
    let u = v
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| MoodleError::InvalidBody(format!(
            "no visible user matches {field}={value}. Note: this site's privacy policy may hide other students' profiles — ids from your conversations, notifications or forum posts still work."
        )))?;
    Ok(json!({
        "user_id": u.get("id").cloned().unwrap_or(Value::Null),
        "username": u.get("username").cloned().unwrap_or(Value::Null),
        "fullname": u.get("fullname").cloned().unwrap_or(Value::Null),
        "email": u.get("email").cloned().unwrap_or(Value::Null),
        "idnumber": u.get("idnumber").cloned().unwrap_or(Value::Null),
        "country": u.get("country").cloned().unwrap_or(Value::Null),
        "city": u.get("city").cloned().unwrap_or(Value::Null),
        "description": u.get("description").and_then(Value::as_str)
            .map(html_to_markdown),
        "first_access": unix_to_iso(u.get("firstaccess").cloned().unwrap_or(Value::Null)),
        "profile_image": u.get("profileimageurl").cloned().unwrap_or(Value::Null),
    }))
}

/// Search users. `wsfunction` varies by Moodle version:
/// `core_user_get_users` (needs user-listing capability) or
/// `core_message_message_search_users` (3.x mobile service). Both may be
/// permission-denied for students — the tool error says so plainly.
pub async fn search_users(
    client: &MoodleClient,
    wsfunction: &str,
    search: Option<&str>,
    limit: u32,
) -> Result<Value, MoodleError> {
    let params = if wsfunction == "core_user_get_users" {
        let criteria = match search {
            Some(q) => json!([{ "key": "search", "value": q }]),
            None => json!([]),
        };
        json!({ "criteria": criteria, "limitnum": limit })
    } else {
        let mut p = json!({});
        if let Some(q) = search {
            p["searchtext"] = json!(q);
        }
        p["limitnum"] = json!(limit);
        p
    };
    let v: Value = client.call(wsfunction, &params).await.map_err(|e| {
        MoodleError::Config(format!(
            "user search is unavailable for this account ({e}). Use known ids from \
             conversations, notifications or forum posts with get_user instead."
        ))
    })?;
    let users: Vec<Value> = v
        .get("users")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|u| {
            json!({
                "user_id": u.get("id").cloned().unwrap_or(Value::Null),
                "username": u.get("username").cloned().unwrap_or(Value::Null),
                "fullname": u.get("fullname").cloned().unwrap_or(Value::Null),
                "email": u.get("email").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({ "count": users.len(), "users": users }))
}
