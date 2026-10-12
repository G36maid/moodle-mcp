use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{strip_html, truncate_chars, unix_to_iso};

/// Shape an assignment record from `mod_assign_get_assignments`.
fn shape_assignment(a: &Value, course: &Value) -> Value {
    json!({
        "assignment_id": a.get("id").cloned().unwrap_or(Value::Null),
        "cmid": a.get("cmid").cloned().unwrap_or(Value::Null),
        "course_id": a.get("course").cloned().unwrap_or(Value::Null),
        "course": course.get("fullname").cloned().unwrap_or(Value::Null),
        "name": a.get("name").cloned().unwrap_or(Value::Null),
        "due": unix_to_iso(a.get("duedate").cloned().unwrap_or(Value::Null)),
        "due_ts": a.get("duedate").cloned().unwrap_or(Value::Null),
        "opens": unix_to_iso(a.get("allowsubmissionsfromdate").cloned().unwrap_or(Value::Null)),
        "cutoff": unix_to_iso(a.get("cutoffdate").cloned().unwrap_or(Value::Null)),
        "grading_due": unix_to_iso(a.get("gradingduedate").cloned().unwrap_or(Value::Null)),
        "draft_submissions_enabled": a.get("submissiondraft").cloned().unwrap_or(Value::Null),
        "nosubmissions": a.get("nosubmissions").cloned().unwrap_or(Value::Null),
        "team_submission": a.get("teamsubmission").cloned().unwrap_or(Value::Null),
        "max_attempts": a.get("maxattempts").cloned().unwrap_or(Value::Null),
        "requiresubmissionstatement": a.get("requiresubmissionstatement").cloned().unwrap_or(Value::Null),
        "description": a.get("intro").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 1200)),
    })
}

/// `mod_assign_get_assignments` — assignments, optionally restricted to
/// specific courses. When `course_ids` is empty, all enrolled courses are used.
pub async fn get_assignments(
    client: &MoodleClient,
    course_ids: &[i64],
) -> Result<Value, MoodleError> {
    let params = if course_ids.is_empty() {
        json!({})
    } else {
        json!({ "courseids": course_ids })
    };
    let v: Value = client.call("mod_assign_get_assignments", &params).await?;
    let mut flat: Vec<Value> = Vec::new();
    if let Some(courses) = v.get("courses").and_then(Value::as_array) {
        for course in courses {
            if let Some(assignments) = course.get("assignments").and_then(Value::as_array) {
                for a in assignments {
                    flat.push(shape_assignment(a, course));
                }
            }
        }
    }
    flat.sort_by_key(|a| {
        a.get("due")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "9999".into())
    });
    Ok(json!({ "count": flat.len(), "assignments": flat }))
}

/// `mod_assign_get_submission_status` — current submission + feedback state.
pub async fn get_assignment_status(
    client: &MoodleClient,
    assignment_id: i64,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call("mod_assign_get_submission_status", &json!({ "assignid": assignment_id }))
        .await?;
    let a = crate::util::first_of(&v, "assignments")
        .ok_or_else(|| MoodleError::InvalidBody("empty submission status".into()))?;

    let submission = a
        .get("submissions")
        .and_then(Value::as_array)
        .and_then(|s| s.first())
        .map(|s| {
            json!({
                "submission_id": s.get("id").cloned().unwrap_or(Value::Null),
                "status": s.get("status").cloned().unwrap_or(Value::Null),
                "created": unix_to_iso(s.get("timecreated").cloned().unwrap_or(Value::Null)),
                "modified": unix_to_iso(s.get("timemodified").cloned().unwrap_or(Value::Null)),
                "attemptnumber": s.get("attemptnumber").cloned().unwrap_or(Value::Null),
                "plugins": s.get("plugins").cloned().unwrap_or(Value::Null),
            })
        })
        .unwrap_or(Value::Null);

    let feedback = a.get("feedback").map(|f| {
        json!({
            "grade": f.get("grade").map(|g| json!({
                "grade": g.get("grade").cloned().unwrap_or(Value::Null),
                "graded_by": g.get("grader").cloned().unwrap_or(Value::Null),
                "graded_at": unix_to_iso(g.get("gradeddatecreated").cloned().unwrap_or(Value::Null)),
                "attemptnumber": g.get("attemptnumber").cloned().unwrap_or(Value::Null),
            })).unwrap_or(Value::Null),
            "plugins": f.get("plugins").cloned().unwrap_or(Value::Null),
        })
    });

    Ok(json!({
        "assignment_id": assignment_id,
        "submission": submission,
        "gradingstatus": a.get("gradingstatus").cloned().unwrap_or(Value::Null),
        "feedback": feedback,
    }))
}

/// `mod_assign_save_submission` — save an onlinetext submission.
pub async fn submit_assignment_text(
    client: &MoodleClient,
    assignment_id: i64,
    text: &str,
    format: &str,
) -> Result<Value, MoodleError> {
    let fmt = match format {
        "html" => 1,
        "plain" => 2,
        "markdown" => 4,
        _ => 1,
    };
    let itemid = crate::util::unique_item_id();
    let params = json!({
        "assignmentid": assignment_id,
        "plugindata": {
            "onlinetext_editor": {
                "text": text,
                "format": fmt,
                "itemid": itemid
            }
        }
    });
    let v: Value = client.call("mod_assign_save_submission", &params).await?;
    Ok(json!({
        "saved": true,
        "assignment_id": assignment_id,
        "format": format,
        "note": "Text submission saved. If the assignment uses draft submissions, also call submit_assignment_for_grading.",
        "warnings": v.get("warnings").cloned().unwrap_or(json!([]))
    }))
}

/// `mod_assign_save_submission` — submit files previously uploaded to a draft
/// area (`upload_file` returns the item id).
pub async fn submit_assignment_files(
    client: &MoodleClient,
    assignment_id: i64,
    item_id: i64,
) -> Result<Value, MoodleError> {
    let params = json!({
        "assignmentid": assignment_id,
        "plugindata": {
            "files_filemanager": item_id
        }
    });
    let v: Value = client.call("mod_assign_save_submission", &params).await?;
    Ok(json!({
        "saved": true,
        "assignment_id": assignment_id,
        "file_item_id": item_id,
        "note": "File submission saved. If the assignment uses draft submissions, also call submit_assignment_for_grading.",
        "warnings": v.get("warnings").cloned().unwrap_or(json!([]))
    }))
}

/// `mod_assign_submit_for_grading` — finalize a (draft) submission for grading.
pub async fn submit_for_grading(
    client: &MoodleClient,
    assignment_id: i64,
    accept_submission_statement: bool,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "mod_assign_submit_for_grading",
            &json!({
                "assignmentid": assignment_id,
                "acceptsubmissionstatement": accept_submission_statement
            }),
        )
        .await?;
    Ok(json!({
        "submitted_for_grading": true,
        "assignment_id": assignment_id,
        "warnings": v.get("warnings").cloned().unwrap_or(json!([]))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_assignment() {
        let a = json!({
            "id": 5, "cmid": 9, "course": 2, "name": "HW1",
            "duedate": 1_700_000_000, "intro": "<p>Read ch.1</p>"
        });
        let course = json!({"fullname": "Math 101"});
        let s = shape_assignment(&a, &course);
        assert_eq!(s["assignment_id"], json!(5));
        assert_eq!(s["course"], json!("Math 101"));
        assert_eq!(s["due"], json!("2023-11-14T22:13:20Z"));
        assert_eq!(s["description"], json!("Read ch.1"));
    }
}
