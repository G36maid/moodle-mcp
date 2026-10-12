use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{strip_html, truncate_chars, unix_to_iso, now_unix};

/// `core_calendar_get_calendar_events` — calendar events, optionally filtered
/// by courses and a time window.
pub async fn get_calendar_events(
    client: &MoodleClient,
    course_ids: &[i64],
    time_from: Option<i64>,
    time_to: Option<i64>,
    limit: u32,
) -> Result<Value, MoodleError> {
    let mut params = json!({});
    if !course_ids.is_empty() {
        params["courseids"] = json!(course_ids);
    }
    if let Some(t) = time_from {
        params["timefrom"] = json!(t);
    }
    if let Some(t) = time_to {
        params["timeto"] = json!(t);
    }
    let v: Value = client.call("core_calendar_get_calendar_events", &params).await?;

    let mut events: Vec<Value> = v
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        // client-side time filter: Moodle < 4.1 silently ignores timefrom/timeto
        .filter(|e| {
            let ts = e.get("timestart").and_then(Value::as_i64).unwrap_or(i64::MAX);
            if let Some(from) = time_from
                && ts < from {
                    return false;
                }
            if let Some(to) = time_to
                && ts > to {
                    return false;
                }
            true
        })
        .map(|e| {
            let duration = e.get("timeduration").and_then(Value::as_i64).unwrap_or(0);
            let start = e.get("timestart").cloned().unwrap_or(Value::Null);
            let end = e
                .get("timestart")
                .and_then(Value::as_i64)
                .map(|t| json!(t + duration))
                .unwrap_or(Value::Null);
            json!({
                "id": e.get("id").cloned().unwrap_or(Value::Null),
                "name": e.get("name").cloned().unwrap_or(Value::Null),
                "description": e.get("description").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 400)),
                "start": unix_to_iso(start),
                "end": unix_to_iso(end),
                "event_type": e.get("eventtype").cloned().unwrap_or(Value::Null),
                "course_id": e.get("courseid").cloned().unwrap_or(Value::Null),
                "module": e.get("modulename").cloned().unwrap_or(Value::Null),
                "action": e.get("action").filter(|a| !a.is_null()).cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    if events.len() > limit as usize {
        events.truncate(limit as usize);
    }
    Ok(json!({ "count": events.len(), "events": events }))
}

/// Facade: upcoming assignment deadlines across all enrolled courses, sorted
/// by due date, computed from `mod_assign_get_assignments`.
pub async fn get_upcoming_deadlines(
    client: &MoodleClient,
    days_ahead: u32,
) -> Result<Value, MoodleError> {
    let all = super::assignments::get_assignments(client, &[]).await?;
    let now = now_unix();
    let horizon = now + (days_ahead as i64) * 86_400;

    let mut out: Vec<Value> = Vec::new();
    for a in all
        .get("assignments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let due_ts = a.get("due_ts").and_then(Value::as_i64).unwrap_or(i64::MAX);
        if due_ts >= now && due_ts <= horizon {
            out.push(a);
        }
    }
    Ok(json!({
        "days_ahead": days_ahead,
        "count": out.len(),
        "deadlines": out
    }))
}
