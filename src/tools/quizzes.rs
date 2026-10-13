//! Quiz attempts (read-only). By design, the quiz-taking chain
//! (start/data/save/process) is NOT wrapped — see docs/design-decisions.md.

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::unix_to_iso;

/// `mod_quiz_get_user_attempts` — the user's attempts on a quiz (all states;
/// `includepreviews=0` skips teacher preview attempts).
pub async fn get_quiz_attempts(
    client: &MoodleClient,
    quiz_id: i64,
    user_id: i64,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "mod_quiz_get_user_attempts",
            &json!({
                "quizid": quiz_id,
                "userid": user_id,
                "status": "all",
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
