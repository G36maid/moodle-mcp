//! Grades: one tool, two lenses.
//! - `get_grades()` — cross-course summary (`gradereport_overview`)
//! - `get_grades(course_id)` — full grade items of one course
//!   (`gradereport_user_get_grade_items`, incl. non-assignment entries)

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{first_str, html_to_markdown, truncate_chars, unix_to_iso};

/// Cross-course summary via `gradereport_overview_get_course_grades`
/// (verified available on NTNU 3.7).
async fn overview(client: &MoodleClient) -> Result<Value, MoodleError> {
    let v: Value = client
        .call("gradereport_overview_get_course_grades", &json!({}))
        .await?;
    let courses: Vec<Value> = v
        .get("grades")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            json!({
                "course_id": g.get("courseid").cloned().unwrap_or(Value::Null),
                "course": first_str(&g, &["coursename", "course_shortname"])
                    .map(str::to_string).unwrap_or_default(),
                "grade": first_str(&g, &["grade", "rawgrade"])
                    .map(str::to_string).unwrap_or_default(),
            })
        })
        .collect();
    Ok(json!({ "mode": "overview", "count": courses.len(), "courses": courses }))
}

/// Full grade items of one course via `gradereport_user_get_grade_items`.
/// NOTE: `userid` MUST be sent — on 3.7, omitting it makes the function fetch
/// ALL users' grades, which requires a capability students lack
/// (required_capability_exception). With userid it returns the caller's own
/// items on every version.
async fn course_items(
    client: &MoodleClient,
    course_id: i64,
    user_id: i64,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "gradereport_user_get_grade_items",
            &json!({ "courseid": course_id, "userid": user_id }),
        )
        .await?;
    let ug = crate::util::first_of(&v, "usergrades")
        .ok_or_else(|| MoodleError::InvalidBody("no grade data returned".into()))?;

    let items: Vec<Value> = ug
        .get("gradeitems")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            let grade = g
                .get("gradeformatted")
                .filter(|x| !x.is_null())
                .cloned()
                .or_else(|| g.get("graderaw").cloned())
                .unwrap_or(Value::Null);
            json!({
                "item": g.get("itemname").filter(|x| !x.is_null()).cloned()
                    .unwrap_or_else(|| g.get("itemtype").cloned().unwrap_or(Value::Null)),
                "type": g.get("itemtype").cloned().unwrap_or(Value::Null),
                "module": g.get("itemmodule").cloned().unwrap_or(Value::Null),
                // For module items, iteminstance is the activity's own id
                // (e.g. assignment_id for assign) — enables direct chaining.
                "assignment_id": if g.get("itemmodule").and_then(Value::as_str) == Some("assign") {
                    g.get("iteminstance").cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                },
                "grade": grade,
                "range": json!({
                    "min": g.get("grademinformatted").filter(|x| !x.is_null())
                        .cloned().or_else(|| g.get("grademin").cloned()).unwrap_or(Value::Null),
                    "max": g.get("grademaxformatted").filter(|x| !x.is_null())
                        .cloned().or_else(|| g.get("grademax").cloned()).unwrap_or(Value::Null),
                }),
                "weight": g.get("weightformatted").cloned().unwrap_or(Value::Null),
                "feedback": g.get("feedback").and_then(Value::as_str)
                    .map(|t| truncate_chars(&html_to_markdown(t), 800)),
                "graded_at": unix_to_iso(g.get("gradedategrade").cloned().unwrap_or(Value::Null)),
            })
        })
        .collect();

    Ok(json!({
        "mode": "course_items",
        "course_id": ug.get("courseid").cloned().unwrap_or(json!(course_id)),
        "course": ug.get("coursename").cloned().unwrap_or(Value::Null),
        "items": items
    }))
}

/// `get_grades(course_id?)` — omit `course_id` for the cross-course summary.
pub async fn get_grades(
    client: &MoodleClient,
    course_id: Option<i64>,
    user_id: i64,
) -> Result<Value, MoodleError> {
    match course_id {
        Some(id) => course_items(client, id, user_id).await,
        None => overview(client).await,
    }
}
