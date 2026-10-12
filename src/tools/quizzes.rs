use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{strip_html, truncate_chars, unix_to_iso};

/// `mod_quiz_get_quizzes_by_courses` — quizzes in the given courses
/// (all enrolled courses when the list is empty).
pub async fn get_quizzes(client: &MoodleClient, course_ids: &[i64]) -> Result<Value, MoodleError> {
    let params = if course_ids.is_empty() {
        json!({})
    } else {
        json!({ "courseids": course_ids })
    };
    let v: Value = client.call("mod_quiz_get_quizzes_by_courses", &params).await?;
    let quizzes: Vec<Value> = v
        .get("quizzes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|q| {
            json!({
                "quiz_id": q.get("id").cloned().unwrap_or(Value::Null),
                "course_id": q.get("course").cloned().unwrap_or(Value::Null),
                "name": q.get("name").cloned().unwrap_or(Value::Null),
                "opens": unix_to_iso(q.get("timeopen").cloned().unwrap_or(Value::Null)),
                "closes": unix_to_iso(q.get("timeclose").cloned().unwrap_or(Value::Null)),
                "time_limit_secs": q.get("timelimit").cloned().unwrap_or(Value::Null),
                "max_attempts": q.get("attempts").cloned().unwrap_or(Value::Null),
                "grading_method": q.get("grademethod").cloned().unwrap_or(Value::Null),
                "intro": q.get("intro").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 300)),
            })
        })
        .collect();
    Ok(json!({ "count": quizzes.len(), "quizzes": quizzes }))
}

/// `mod_quiz_get_user_attempts` — attempts of a user on a quiz.
pub async fn get_quiz_attempts(
    client: &MoodleClient,
    quiz_id: i64,
    user_id: i64,
    status: &str,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "mod_quiz_get_user_attempts",
            &json!({
                "quizid": quiz_id,
                "userid": user_id,
                "status": status,
                "includepreviews": 0
            }),
        )
        .await?;
    let attempts: Vec<Value> = v
        .get("attempts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|a| {
            json!({
                "attempt_id": a.get("id").cloned().unwrap_or(Value::Null),
                "attempt_number": a.get("attempt").cloned().unwrap_or(Value::Null),
                "state": a.get("state").cloned().unwrap_or(Value::Null),
                "started": unix_to_iso(a.get("timestart").cloned().unwrap_or(Value::Null)),
                "finished": unix_to_iso(a.get("timefinish").cloned().unwrap_or(Value::Null)),
                "sum_grades": a.get("sumgrades").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({ "quiz_id": quiz_id, "count": attempts.len(), "attempts": attempts }))
}
