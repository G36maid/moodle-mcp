use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{strip_html, truncate_chars, unix_to_iso};

/// `gradereport_user_get_grade_items` — detailed grade items for a user in a
/// course. Falls back to the raw grade when no formatted grade is present.
pub async fn get_grades(
    client: &MoodleClient,
    course_id: i64,
    user_id: i64,
) -> Result<Value, MoodleError> {
    let mut params = json!({ "courseid": course_id });
    if user_id > 0 {
        params["userid"] = json!(user_id);
    }
    let v: Value = client
        .call("gradereport_user_get_grade_items", &params)
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
            let min = g
                .get("grademinformatted")
                .filter(|x| !x.is_null())
                .cloned()
                .or_else(|| g.get("grademin").cloned())
                .unwrap_or(Value::Null);
            let max = g
                .get("grademaxformatted")
                .filter(|x| !x.is_null())
                .cloned()
                .or_else(|| g.get("grademax").cloned())
                .unwrap_or(Value::Null);
            json!({
                "item": g.get("itemname").filter(|x| !x.is_null()).cloned()
                    .unwrap_or_else(|| g.get("itemtype").cloned().unwrap_or(Value::Null)),
                "type": g.get("itemtype").cloned().unwrap_or(Value::Null),
                "module": g.get("itemmodule").cloned().unwrap_or(Value::Null),
                "grade": grade,
                "range": json!({"min": min, "max": max}),
                "weight": g.get("weightformatted").cloned().unwrap_or(Value::Null),
                "feedback": g.get("feedback").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 800)),
                "graded_at": unix_to_iso(g.get("gradedategrade").cloned().unwrap_or(Value::Null)),
                "hidden": g.get("hidden").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    Ok(json!({
        "course_id": ug.get("courseid").cloned().unwrap_or(json!(course_id)),
        "course": ug.get("coursename").cloned().unwrap_or(Value::Null),
        "user_id": ug.get("user").and_then(|u| u.get("id")).cloned().unwrap_or(Value::Null),
        "items": items
    }))
}
