use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{strip_html, truncate_chars, unix_to_iso};

/// Get recent notifications. `wsfunction` varies by Moodle version:
/// `core_message_get_notifications` (3.9+ mobile service) or
/// `core_message_get_messages` with type=notifications (Moodle 3.x).
pub async fn get_notifications(
    client: &MoodleClient,
    wsfunction: &str,
    user_id: i64,
    read: Option<bool>,
    limit: u32,
) -> Result<Value, MoodleError> {
    let legacy = wsfunction == "core_message_get_messages";
    let mut params = json!({
        "useridto": user_id,
        "limitnum": limit
    });
    if legacy {
        params["type"] = json!("notifications");
        params["newestfirst"] = json!(true);
        // legacy API filters by read state 0/1; only send when requested
        if let Some(r) = read {
            params["read"] = json!(if r { 1 } else { 0 });
        }
    } else {
        params["newestfirst"] = json!(true);
        if let Some(r) = read {
            params["read"] = json!(r);
        }
    }
    let v: Value = client.call(wsfunction, &params).await?;
    let items: Vec<Value> = v
        .get("list")
        .or_else(|| v.get("messages"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|n| {
            json!({
                "id": n.get("id").cloned().unwrap_or(Value::Null),
                "subject": n.get("subject").cloned().unwrap_or(Value::Null),
                "from": n.get("userfromfullname").cloned().unwrap_or(Value::Null),
                "from_user_id": n.get("useridfrom").cloned().unwrap_or(Value::Null),
                "text": n.get("text").or_else(|| n.get("smallmessage")).or_else(|| n.get("fullmessage"))
                    .and_then(Value::as_str)
                    .map(|t| truncate_chars(&strip_html(t), 500)),
                "timecreated": unix_to_iso(n.get("timecreated").cloned().unwrap_or(Value::Null)),
                "read": n.get("read").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({ "count": items.len(), "notifications": items }))
}

/// `core_message_get_conversations` — messaging conversations for the user.
pub async fn get_conversations(
    client: &MoodleClient,
    user_id: i64,
    limit: u32,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "core_message_get_conversations",
            &json!({ "userid": user_id, "limitnum": limit }),
        )
        .await?;
    let conversations: Vec<Value> = v
        .get("conversations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            // Modern Moodle: `lastmessage` (array or object) + `timemodified`.
            // Moodle 3.7: no lastmessage/timemodified; last entries live in
            // the `messages` array, and the conversation kind is `type`.
            let last_msg = c
                .get("lastmessage")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .or_else(|| {
                    c.get("messages")
                        .and_then(Value::as_array)
                        .and_then(|a| a.last())
                })
                .cloned()
                .unwrap_or(Value::Null);
            let last_activity = if c.get("timemodified").is_some() {
                unix_to_iso(c.get("timemodified").cloned().unwrap_or(Value::Null))
            } else {
                unix_to_iso(last_msg.get("timecreated").cloned().unwrap_or(Value::Null))
            };
            json!({
                "id": c.get("id").cloned().unwrap_or(Value::Null),
                "name": c.get("name").cloned().unwrap_or(Value::Null),
                "type": c.get("subtype").cloned().unwrap_or_else(|| c.get("type").cloned().unwrap_or(Value::Null)),
                "member_count": c.get("membercount").cloned().unwrap_or(Value::Null),
                "unread": c.get("unreadcount").cloned().unwrap_or(Value::Null),
                "last_message": if last_msg.is_null() { Value::Null } else { json!({
                    "text": last_msg.get("text").or_else(|| last_msg.get("smallmessage")).or_else(|| last_msg.get("fullmessage"))
                        .and_then(Value::as_str)
                        .map(|t| truncate_chars(&strip_html(t), 300)),
                    "from_user_id": last_msg.get("useridfrom").cloned().unwrap_or(Value::Null),
                }) },
                "last_activity": last_activity,
            })
        })
        .collect();
    Ok(json!({
        "count": conversations.len(),
        "unread_total": v.get("unreadcount").cloned().unwrap_or(Value::Null),
        "conversations": conversations
    }))
}

/// `core_message_send_instant_messages` — send a message to a user.
pub async fn send_message(
    client: &MoodleClient,
    to_user_id: i64,
    text: &str,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "core_message_send_instant_messages",
            &json!({ "messages": [ { "touserid": to_user_id, "text": text, "textformat": 1 } ] }),
        )
        .await?;
    let first = v.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
    if let Some(err) = first.get("errormessage").and_then(Value::as_str) {
        return Err(MoodleError::InvalidBody(format!(
            "Moodle rejected the message: {err}"
        )));
    }
    Ok(json!({
        "sent": true,
        "to_user_id": to_user_id,
        "message_id": first.get("msgid").cloned().unwrap_or(Value::Null),
        "sent_at": unix_to_iso(first.get("timecreated").cloned().unwrap_or(Value::Null)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_errormessage() {
        let first = json!({"msgid": null, "errormessage": "User 1 is not reachable"});
        let err = first.get("errormessage").and_then(Value::as_str).unwrap();
        assert!(err.contains("not reachable"));
    }
}
