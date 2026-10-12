use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{truncate_chars, unix_to_iso};

/// Search users. `wsfunction` varies by Moodle version:
/// `core_user_get_users` (needs user-listing capability) or
/// `core_message_message_search_users` (Moodle 3.x mobile service).
pub async fn search_users(
    client: &MoodleClient,
    wsfunction: &str,
    search: Option<&str>,
    limit: u32,
) -> Result<Value, MoodleError> {
    let (params, users_key): (Value, &str) = if wsfunction == "core_user_get_users" {
        let criteria = match search {
            Some(q) => json!([{ "key": "search", "value": q }]),
            None => json!([]),
        };
        (json!({ "criteria": criteria, "limitnum": limit }), "users")
    } else {
        let mut p = json!({});
        if let Some(q) = search {
            p["searchtext"] = json!(q);
        }
        p["limitnum"] = json!(limit);
        (p, "users")
    };
    let v: Value = client.call(wsfunction, &params).await?;
    let users: Vec<Value> = v
        .get(users_key)
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

/// `core_user_get_users_by_field` — look up users by username / email / id / idnumber.
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
        .ok_or_else(|| MoodleError::InvalidBody(format!("no user found with {field}={value}")))?;
    Ok(json!({
        "user_id": u.get("id").cloned().unwrap_or(Value::Null),
        "username": u.get("username").cloned().unwrap_or(Value::Null),
        "fullname": u.get("fullname").cloned().unwrap_or(Value::Null),
        "firstname": u.get("firstname").cloned().unwrap_or(Value::Null),
        "lastname": u.get("lastname").cloned().unwrap_or(Value::Null),
        "email": u.get("email").cloned().unwrap_or(Value::Null),
        "idnumber": u.get("idnumber").cloned().unwrap_or(Value::Null),
        "country": u.get("country").cloned().unwrap_or(Value::Null),
        "city": u.get("city").cloned().unwrap_or(Value::Null),
        "description": u.get("description").and_then(Value::as_str).map(|t| truncate_chars(t, 300)),
        "first_access": unix_to_iso(u.get("firstaccess").cloned().unwrap_or(Value::Null)),
        "profile_image": u.get("profileimageurl").cloned().unwrap_or(Value::Null),
    }))
}
