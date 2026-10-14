//! Messaging & notifications.
//!
//! NTNU 3.7 specifics handled internally:
//! - `core_message_get_notifications` is absent on 3.7 → falls back to
//!   `core_message_get_messages?type=notifications` (caller resolves via
//!   `pick_function`)
//! - conversations: `unreadcount` is null on 3.7 → falls back to `isread`;
//!   private chats have an empty `name` → display name taken from `members`

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{html_to_markdown, truncate_chars, unix_to_iso};

/// Recent notifications for the user.
///
/// `read_both_supported` says whether the site's `core_message_get_messages`
/// accepts `read=2` ("both") — Moodle 4.0+ (PARAM_INT); up to 3.11 the
/// parameter is a boolean with no "both" value. When the caller omits
/// `read`, the tool asks for both halves explicitly: one `read=2` call where
/// supported, otherwise two calls (`read=0` + `read=1`) merged by id — the
/// function defaults to "read only" on every version, which would silently
/// drop the unread half.
#[allow(clippy::too_many_arguments)]
pub async fn get_notifications(
    client: &MoodleClient,
    wsfunction: &str,
    user_id: i64,
    read: Option<bool>,
    limit: u32,
    read_both_supported: bool,
) -> Result<Value, MoodleError> {
    let legacy = wsfunction == "core_message_get_messages";
    let popup = wsfunction == "message_popup_get_popup_notifications";
    let mut params = json!({ "useridto": user_id, "limitnum": limit.max(1) });
    if legacy {
        params["type"] = json!("notifications");
        params["newestfirst"] = json!(true);
        if let Some(r) = read {
            params["read"] = json!(if r { 1 } else { 0 });
        }
    } else if popup {
        // message_popup_get_popup_notifications accepts neither `read` nor
        // `newestfirst`; extra params would be rejected with invalidparameter.
    } else {
        params["newestfirst"] = json!(true);
        if let Some(r) = read {
            params["read"] = json!(r);
        }
    }

    let items: Vec<Value> = if legacy && read.is_none() {
        if read_both_supported {
            let mut p = params;
            p["read"] = json!(2);
            extract_notification_items(&client.call(wsfunction, &p).await?)
        } else {
            let mut p_unread = params.clone();
            p_unread["read"] = json!(0);
            let mut p_read = params;
            p_read["read"] = json!(1);
            let unread = extract_notification_items(&client.call(wsfunction, &p_unread).await?);
            let read_only = extract_notification_items(&client.call(wsfunction, &p_read).await?);
            // The read half is queried last, so its state is fresher — put it
            // first so dedupe keeps its copy of any id present in both halves.
            let mut merged = read_only;
            merged.extend(unread);
            // Newest first (matching newestfirst=true), deduped by id in case
            // the two halves overlap server-side.
            merged.sort_by(|a, b| {
                let ta = a.get("timecreated").and_then(Value::as_i64).unwrap_or(0);
                let tb = b.get("timecreated").and_then(Value::as_i64).unwrap_or(0);
                tb.cmp(&ta)
            });
            merged.dedup_by(|a, b| {
                a.get("id")
                    .and_then(Value::as_i64)
                    .is_some_and(|id| b.get("id").and_then(Value::as_i64) == Some(id))
            });
            merged.truncate(limit.max(1) as usize);
            merged
        }
    } else {
        extract_notification_items(&client.call(wsfunction, &params).await?)
    };

    let items: Vec<Value> = items
        .into_iter()
        .map(|n| {
            json!({
                "id": n.get("id").cloned().unwrap_or(Value::Null),
                "subject": n.get("subject").cloned().unwrap_or(Value::Null),
                "from": n.get("userfromfullname").cloned().unwrap_or(Value::Null),
                "from_user_id": n.get("useridfrom").cloned().unwrap_or(Value::Null),
                "text": n.get("text").or_else(|| n.get("smallmessage"))
                    .or_else(|| n.get("fullmessage")).and_then(Value::as_str)
                    .map(|t| truncate_chars(&html_to_markdown(t), 500)),
                "timecreated": unix_to_iso(n.get("timecreated").cloned().unwrap_or(Value::Null)),
                // core_message_get_messages items carry `timeread` (null while
                // unread) instead of a `read` key — derive the flag from it.
                "read": read_flag(&n),
            })
        })
        .collect();
    Ok(json!({ "count": items.len(), "notifications": items }))
}

/// Pull the notification/message array out of any response shape
/// (`list` / `messages` / `notifications`).
fn extract_notification_items(v: &Value) -> Vec<Value> {
    v.get("list")
        .or_else(|| v.get("messages"))
        .or_else(|| v.get("notifications"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The `read` state of an item: taken from `read` when present (popup
/// notifications), otherwise derived from `timeread` (null/0 = unread).
fn read_flag(n: &Value) -> Value {
    if let Some(r) = n.get("read")
        && !r.is_null()
    {
        return r.clone();
    }
    match n.get("timeread") {
        Some(t) if !t.is_null() && t.as_i64() != Some(0) => json!(true),
        Some(_) => json!(false),
        None => Value::Null,
    }
}

/// `core_message_get_conversations` — the user's conversations.
///
/// Private chats (empty `name`) get their display name from the other
/// member; unread falls back to `isread` when `unreadcount` is null (3.7).
pub async fn get_conversations(
    client: &MoodleClient,
    user_id: i64,
    limit: u32,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "core_message_get_conversations",
            &json!({ "userid": user_id, "limitnum": limit.max(1) }),
        )
        .await?;

    let conversations: Vec<Value> = v
        .get("conversations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let members: Vec<Value> = c
                .get("members")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|m| {
                    json!({
                        "id": m.get("id").cloned().unwrap_or(Value::Null),
                        "fullname": m.get("fullname").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect();

            // Display name: explicit name, else the other member(s) of a
            // private chat (3.7 leaves `name` empty there).
            let name = match c.get("name").and_then(Value::as_str) {
                Some(n) if !n.is_empty() => Value::String(n.to_string()),
                _ => {
                    let others: Vec<String> = members
                        .iter()
                        .filter(|m| m.get("id").and_then(Value::as_i64) != Some(user_id))
                        .filter_map(|m| m.get("fullname").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect();
                    if others.is_empty() {
                        Value::Null
                    } else {
                        Value::String(others.join(", "))
                    }
                }
            };

            // Modern versions expose `lastmessage`; 3.7 puts the last entries
            // in `messages`. Unread: `unreadcount`, falling back to `isread`.
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
                "name": name,
                "type": c.get("subtype").cloned()
                    .unwrap_or_else(|| c.get("type").cloned().unwrap_or(Value::Null)),
                "member_count": c.get("membercount").cloned().unwrap_or(Value::Null),
                "unread": c.get("unreadcount").cloned()
                    .unwrap_or_else(|| c.get("isread").cloned()
                        .map(|r| json!(if r.as_bool() == Some(false) { 1 } else { 0 }))
                        .unwrap_or(Value::Null)),
                "last_message": if last_msg.is_null() { Value::Null } else { json!({
                    "text": last_msg.get("text").or_else(|| last_msg.get("smallmessage"))
                        .and_then(Value::as_str)
                        .map(|t| truncate_chars(&html_to_markdown(t), 300)),
                    "from_user_id": last_msg.get("useridfrom").cloned().unwrap_or(Value::Null),
                }) },
                "last_activity": last_activity,
                "members": members,
            })
        })
        .collect();
    Ok(json!({
        "count": conversations.len(),
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
    let first = v
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or(Value::Null);
    if let Some(err) = first.get("errormessage").and_then(Value::as_str) {
        return Err(MoodleError::InvalidBody(format!(
            "Moodle rejected the message: {err}"
        )));
    }
    Ok(json!({
        "sent": true,
        "to_user_id": to_user_id,
        "message_id": first.get("msgid").cloned().unwrap_or(Value::Null),
    }))
}
