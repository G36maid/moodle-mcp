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
///
/// 3.7 regression guard: `core_user_get_users` is often *enabled* on 3.7 but
/// rejects the `search` criteria key (added in 3.8) with `invalidparameter`.
/// On that error we transparently retry with `core_message_message_search_users`
/// (available since 3.2 in the mobile service) instead of reporting a
/// misleading permission problem.
pub async fn search_users(
    client: &MoodleClient,
    wsfunction: &str,
    user_id: i64,
    search: Option<&str>,
    limit: u32,
) -> Result<Value, MoodleError> {
    let v = search_call(client, wsfunction, user_id, search, limit).await;
    let v = match v {
        Err(MoodleError::Api { ref errorcode, .. })
            if errorcode == "invalidparameter" && wsfunction == "core_user_get_users" =>
        {
            search_call(
                client,
                "core_message_message_search_users",
                user_id,
                search,
                limit,
            )
            .await
        }
        other => other,
    };
    let v: Value = v.map_err(|e| {
        MoodleError::Config(format!(
            "user search is unavailable for this account ({e}). Use known ids from \
             conversations, notifications or forum posts with get_user instead."
        ))
    })?;
    let users: Vec<Value> = v
        .get("users")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            // core_message_message_search_users answers with
            // {contacts, noncontacts} (verified live on 3.7; 4.5 externallib
            // defines the same shape) — merge both halves:
            // https://github.com/moodle/moodle/blob/v4.5.4/message/externallib.php#L1181-L1185
            ["contacts", "noncontacts"]
                .iter()
                .flat_map(|k| {
                    v.get(*k)
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                })
                .collect()
        })
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

/// One search attempt against a specific WS function.
async fn search_call(
    client: &MoodleClient,
    wsfunction: &str,
    user_id: i64,
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
        // core_message_message_search_users requires `userid` and `search`
        // (verified identical on 3.7 and 4.5+ — `searchtext` is rejected
        // with invalidparameter) and only lets the account search as
        // itself, so always send the token owner's id:
        // https://github.com/moodle/moodle/blob/v4.5.4/message/externallib.php#L1123
        // https://github.com/moodle/moodle/blob/v3.7.0/message/externallib.php#L1571
        json!({
            "userid": user_id,
            "search": search.unwrap_or(""),
            "limitfrom": 0,
            "limitnum": limit,
        })
    };
    client.call(wsfunction, &params).await
}
