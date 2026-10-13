//! `get_schedule` — the unified timeline (replaces the old
//! `get_calendar_events` + `get_upcoming_deadlines` pair).
//!
//! A single WS call (`core_calendar_get_calendar_events`) with **all**
//! enrolled course ids covers activity due dates, teacher-created events and
//! personal events in one response — on every Moodle version from 3.7 to 5.x
//! (nested param shape is identical; module events require explicit
//! courseids, verified empirically).

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{now_unix, strip_empty_warnings, unix_to_iso};

/// Default horizon when `time_to` is omitted: the next 30 days.
const DEFAULT_DAYS_AHEAD: i64 = 30;

/// Hints attached to `get_schedule` when parameters are omitted (shared with
/// the tool wrapper in `mod.rs`).
pub const HINT_TIME: &str =
    "Time range defaults to the next 30 days; pass time_from/time_to to query other ranges.";
pub const HINT_COURSE: &str = "Defaults to all enrolled courses; pass course_ids to filter.";
pub const HINT_REVERSED: &str = "time_from is after time_to — the window is empty, so no events match. Swap the two values to get results.";

pub async fn get_schedule(
    client: &MoodleClient,
    course_ids: &[i64],
    time_from: Option<i64>,
    time_to: Option<i64>,
) -> Result<(Value, Vec<String>), MoodleError> {
    let from = time_from.unwrap_or_else(now_unix);
    let to = time_to.unwrap_or_else(|| now_unix() + DEFAULT_DAYS_AHEAD * 86_400);
    let mut hints = Vec::new();
    if from > to {
        hints.push(HINT_REVERSED.to_string());
    }

    let mut params = json!({
        "events": { "courseids": course_ids },
        "options": { "timestart": from, "timeend": to },
    });
    if course_ids.is_empty() {
        // No courses to filter on: omit the nested key entirely.
        params = json!({ "options": { "timestart": from, "timeend": to } });
    }

    let raw: Value = client
        .call("core_calendar_get_calendar_events", &params)
        .await?;

    let mut events: Vec<Value> = raw
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|e| {
            json!({
                "time": unix_to_iso(e.get("timestart").cloned().unwrap_or(Value::Null)),
                "name": e.get("name").cloned().unwrap_or(Value::Null),
                "type": e.get("eventtype").cloned().unwrap_or(Value::Null),
                "source": e.get("modulename").cloned().unwrap_or(Value::Null),
                "course_id": e.get("courseid").cloned().unwrap_or(Value::Null),
                "event_id": e.get("id").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    events.sort_by_key(|e| {
        // ISO-8601 strings sort chronologically.
        e.get("time")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default()
    });

    Ok((
        strip_empty_warnings(json!({
            "count": events.len(),
            "from": crate::util::unix_to_iso(json!(from)),
            "to": crate::util::unix_to_iso(json!(to)),
            "schedule": events,
        })),
        hints,
    ))
}
