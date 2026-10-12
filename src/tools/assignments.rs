//! Assignments: one read tool (`get_assignment` — everything about a single
//! assignment) and one write tool (`submit_assignment` — text, files,
//! finalize; the whole submission chain in one call).

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{decode_entities, html_to_markdown, truncate_chars, unique_item_id, unix_to_iso};

/// Extract file references from an assign plugin's `fileareas`.
fn files_from_plugin(plugin: &Value) -> Vec<Value> {
    plugin
        .get("fileareas")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|area| {
            area.get("files")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .map(|f| {
            json!({
                "filename": f.get("filename").cloned().unwrap_or(Value::Null),
                "fileurl": f.get("fileurl").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
}

/// Pull the text of the first editor field of a plugin (onlinetext answer,
/// teacher's feedback comment…).
fn text_from_plugin(plugin: &Value) -> Option<String> {
    plugin
        .get("editorfields")
        .and_then(Value::as_array)
        .and_then(|fields| fields.first())
        .and_then(|f| f.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// `get_assignment` — everything about a single assignment in one call:
/// metadata, full description, attachments, current submission (incl. its
/// onlinetext and files), grade and feedback. Rendered as a markdown
/// document; the structured channel mirrors the same data.
pub async fn get_assignment(
    client: &MoodleClient,
    assignment_id: i64,
) -> Result<(String, Value), MoodleError> {
    // One broad call, filtered server-side here: mod_assign has no
    // "get one assignment by id" function on 3.7.
    let all: Value = client
        .call("mod_assign_get_assignments", &json!({}))
        .await?;
    let (assignment, course): (Value, Value) = all
        .get("courses")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|course| {
            course
                .get("assignments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(move |a| (a, course.clone()))
        })
        .find(|(a, _)| a.get("id").and_then(Value::as_i64) == Some(assignment_id))
        .ok_or_else(|| {
            MoodleError::InvalidBody(format!(
                "assignment {assignment_id} not found among your enrolled courses' assignments"
            ))
        })?;

    let status: Value = client
        .call(
            "mod_assign_get_submission_status",
            &json!({ "assignid": assignment_id }),
        )
        .await?;

    // Two response shapes exist: modern Moodle returns an `assignments` array
    // (with submissions/feedback nested); 3.7 returns `lastattempt{}` and a
    // top-level `feedback`. Normalize both into (submission, feedback,
    // gradingstatus).
    let (submission, feedback, gradingstatus) =
        if let Some(a) = crate::util::first_of(&status, "assignments") {
            (
                crate::util::first_of(a, "submissions")
                    .cloned()
                    .unwrap_or(Value::Null),
                a.get("feedback").cloned().unwrap_or(Value::Null),
                a.get("gradingstatus").cloned().unwrap_or(Value::Null),
            )
        } else {
            let la = status.get("lastattempt").cloned().unwrap_or(Value::Null);
            (
                la.get("submission").cloned().unwrap_or(Value::Null),
                status.get("feedback").cloned().unwrap_or(Value::Null),
                la.get("gradingstatus").cloned().unwrap_or(Value::Null),
            )
        };
    if submission.is_null() && feedback.is_null() {
        return Err(MoodleError::InvalidBody("empty submission status".into()));
    }
    let _graded = matches!(
        gradingstatus.as_str(),
        Some("graded") | Some("graded_followup_submitted") | Some("graded_followup_edit")
    );

    // --- submission ---
    let mut sub_files: Vec<Value> = Vec::new();
    let mut onlinetext: Option<String> = None;
    for plugin in submission
        .get("plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let ptype = plugin
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if ptype == "onlinetext" {
            onlinetext = text_from_plugin(&plugin).map(|t| decode_entities(&t));
        } else {
            sub_files.extend(files_from_plugin(&plugin));
        }
    }

    // --- feedback ---
    let grade = feedback
        .get("grade")
        .and_then(|g| g.get("grade"))
        .cloned()
        .unwrap_or(Value::Null);
    let mut fb_comment: Option<String> = None;
    let mut fb_files: Vec<Value> = Vec::new();
    for plugin in feedback
        .get("plugins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let ptype = plugin
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if ptype.contains("comment") {
            fb_comment = text_from_plugin(&plugin).map(|t| decode_entities(&t));
        } else if ptype.contains("file") {
            fb_files.extend(files_from_plugin(&plugin));
        }
    }
    let graded = !grade.is_null();

    // --- description & attachments ---
    let intro_md = assignment
        .get("intro")
        .and_then(Value::as_str)
        .map(html_to_markdown)
        .unwrap_or_default();
    let attachments: Vec<Value> = assignment
        .get("introattachments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            json!({
                "filename": f.get("filename").cloned().unwrap_or(Value::Null),
                "fileurl": f.get("fileurl").cloned().unwrap_or(Value::Null),
                "filesize": f.get("filesize").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    let draft_required = assignment.get("submissiondraft").and_then(Value::as_i64) == Some(1);
    let submission_status = submission
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if draft_required {
            "not started"
        } else {
            "no submission yet"
        })
        .to_string();

    let course_name = course.get("fullname").cloned().unwrap_or(Value::Null);
    let course_id = assignment.get("course").cloned().unwrap_or(Value::Null);
    let attempt_number = submission
        .get("attemptnumber")
        .cloned()
        .unwrap_or(Value::Null);
    let prev_count = 0; // 3.7 的 submission_status 不含 previousattempts

    // --- structured ---
    let structured = json!({
        "assignment_id": assignment_id,
        "cmid": assignment.get("cmid").cloned().unwrap_or(Value::Null),
        "course_id": course_id,
        "course": course_name,
        "name": assignment.get("name").cloned().unwrap_or(Value::Null),
        "times": {
            "opens": unix_to_iso(assignment.get("allowsubmissionsfromdate").cloned().unwrap_or(Value::Null)),
            "due": unix_to_iso(assignment.get("duedate").cloned().unwrap_or(Value::Null)),
            "cutoff": unix_to_iso(assignment.get("cutoffdate").cloned().unwrap_or(Value::Null)),
        },
        "draft_required": draft_required,
        "description_markdown": intro_md,
        "attachments": attachments,
        "submission": {
            "status": submission_status,
            "attempt_number": attempt_number,
            "previous_attempts": prev_count,
            "onlinetext": onlinetext,
            "files": sub_files,
        },
        "graded": graded,
        "grade": grade,
        "feedback_comment": fb_comment,
        "feedback_files": fb_files,
    });

    // --- markdown ---
    let iso = |v: &Value| v.as_str().unwrap_or("—").to_string();
    let mut md = String::new();
    md.push_str(&format!(
        "# {} — assignment_id: {assignment_id}\n\n",
        assignment
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
    ));
    md.push_str(&format!(
        "**Course**: {} (course_id: {course_id})\n\n",
        course_name.as_str().unwrap_or("?")
    ));
    md.push_str(&format!(
        "**Opens**: {} · **Due**: {} · **Cutoff**: {}\n\n",
        iso(&structured["times"]["opens"]),
        iso(&structured["times"]["due"]),
        iso(&structured["times"]["cutoff"]),
    ));
    md.push_str(&format!(
        "**Submission**: {} · **Draft required**: {} · **Attempts**: {} (previous: {prev_count}) · **Graded**: {}\n\n",
        submission_status,
        if draft_required { "yes — call submit_assignment with finalize=true to turn in" } else { "no" },
        attempt_number,
        graded,
    ));

    if !intro_md.is_empty() {
        md.push_str("## Description\n\n");
        md.push_str(&truncate_chars(&intro_md, 4000));
        md.push_str("\n\n");
    }
    if !attachments.is_empty() {
        md.push_str("## Attachments (fetch with fetch_file)\n");
        for a in &attachments {
            md.push_str(&format!(
                "- {} — {}\n",
                a["filename"].as_str().unwrap_or("?"),
                a["fileurl"].as_str().unwrap_or("?"),
            ));
        }
        md.push('\n');
    }
    md.push_str("## My submission\n\n");
    if let Some(text) = &onlinetext {
        md.push_str(&format!("**Online text**:\n\n{text}\n\n"))
    }
    if !sub_files.is_empty() {
        md.push_str("**Files**:\n");
        for f in &sub_files {
            md.push_str(&format!(
                "- {} — {}\n",
                f["filename"].as_str().unwrap_or("?"),
                f["fileurl"].as_str().unwrap_or("")
            ));
        }
        md.push('\n');
    }
    if graded {
        md.push_str("## Grade & feedback\n\n");
        md.push_str(&format!("**Grade**: {}\n\n", grade));
        if let Some(c) = &fb_comment {
            md.push_str(&format!("**Teacher's comment**: {c}\n\n"));
        }
        if !fb_files.is_empty() {
            md.push_str("**Feedback files**:\n");
            for f in &fb_files {
                md.push_str(&format!(
                    "- {} — {}\n",
                    f["filename"].as_str().unwrap_or("?"),
                    f["fileurl"].as_str().unwrap_or("")
                ));
            }
        }
    } else if submission.is_null() {
        md.push_str("_Not submitted yet._\n");
    }

    Ok((md, structured))
}

/// `submit_assignment` — the single write: save an onlinetext and/or draft
/// files as the submission, optionally finalizing it for grading.
/// Returns pure results (read back with `get_assignment`).
pub async fn submit_assignment(
    client: &MoodleClient,
    assignment_id: i64,
    text: Option<&str>,
    item_id: Option<i64>,
    finalize: bool,
) -> Result<Value, MoodleError> {
    if text.is_none() && item_id.is_none() {
        return Err(MoodleError::Config(
            "provide text (online text) and/or item_id (draft files) to submit".into(),
        ));
    }
    let mut plugindata = json!({});
    if let Some(t) = text {
        plugindata["onlinetext_editor"] = json!({
            "text": t,
            "format": 1, // FORMAT_HTML
            "itemid": unique_item_id(),
        });
    }
    if let Some(id) = item_id {
        plugindata["files_filemanager"] = json!(id);
    }
    let _: Value = client
        .call(
            "mod_assign_save_submission",
            &json!({ "assignmentid": assignment_id, "plugindata": plugindata }),
        )
        .await?;

    let mut out = json!({
        "saved": true,
        "assignment_id": assignment_id,
        "submitted_for_grading": false,
    });
    if finalize {
        let _: Value = client
            .call(
                "mod_assign_submit_for_grading",
                &json!({
                    "assignmentid": assignment_id,
                    "acceptsubmissionstatement": true
                }),
            )
            .await?;
        out["submitted_for_grading"] = json!(true);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_files_and_text_from_plugins() {
        let st = json!({
            "submissions": [{
                "plugins": [
                    {"type": "onlinetext", "editorfields": [{"name": "onlinetext", "text": "<p>my answer</p>"}]},
                    {"type": "file", "fileareas": [{"area": "submission_files", "files": [
                        {"filename": "a.pdf", "fileurl": "https://x/pluginfile.php/1"}
                    ]}]}
                ]
            }]
        });
        let sub = crate::util::first_of(&st, "submissions").unwrap();
        let mut files = Vec::new();
        let mut text = None;
        for plugin in sub
            .get("plugins")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let ptype = plugin
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if ptype == "onlinetext" {
                text = text_from_plugin(&plugin);
            } else {
                files.extend(files_from_plugin(&plugin));
            }
        }
        assert_eq!(text.as_deref(), Some("<p>my answer</p>"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["filename"], json!("a.pdf"));
    }

    #[test]
    fn finds_assignment_across_courses() {
        let all = json!({"courses": [
            {"fullname": "A", "assignments": [{"id": 1, "name": "HW1"}]},
            {"fullname": "B", "assignments": [{"id": 2, "name": "HW2"}]}
        ]});
        let found = all
            .get("courses")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .flat_map(|c| {
                c.get("assignments")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .find(|a| a.get("id").and_then(Value::as_i64) == Some(2));
        assert_eq!(found.unwrap()["name"], json!("HW2"));
    }
}
