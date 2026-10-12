//! Courses: the entry point (`list_courses`) and the course map
//! (`get_course_overview` — 5 WS sources merged into one markdown document).

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{decode_entities, dig, html_to_markdown, truncate_chars, unix_to_iso};

/// `list_courses` — courses the token owner is enrolled in, optionally
/// filtered by name (client-side, case-insensitive).
pub async fn list_courses(
    client: &MoodleClient,
    userid: i64,
    search: Option<&str>,
) -> Result<Value, MoodleError> {
    let raw: Value = client
        .call("core_enrol_get_users_courses", &json!({ "userid": userid }))
        .await?;

    let needle = search.map(|s| s.to_lowercase());
    let courses: Vec<Value> = raw
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| match &needle {
            Some(q) => ["fullname", "shortname", "displayname"].iter().any(|k| {
                c.get(k)
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.to_lowercase().contains(q))
            }),
            None => true,
        })
        .map(|c| {
            json!({
                "course_id": c.get("id").cloned().unwrap_or(Value::Null),
                "fullname": c.get("fullname").cloned().unwrap_or(Value::Null),
                "shortname": c.get("shortname").cloned().unwrap_or(Value::Null),
                "progress": c.get("progress").cloned().unwrap_or(Value::Null),
                "startdate": unix_to_iso(c.get("startdate").cloned().unwrap_or(Value::Null)),
                "enddate": unix_to_iso(c.get("enddate").cloned().unwrap_or(Value::Null)),
            })
        })
        .collect();
    Ok(json!({ "count": courses.len(), "courses": courses }))
}

/// One module entry from `core_course_get_contents`, normalized.
fn shape_module(m: &Value, assignments_by_cmid: &std::collections::HashMap<i64, Value>) -> Value {
    let modname = m.get("modname").cloned().unwrap_or(Value::Null);
    let cmid = m.get("id").cloned().unwrap_or(Value::Null);
    let mut entry = json!({
        "module_id": cmid,
        "type": modname,
        "name": m.get("name").cloned().unwrap_or(Value::Null),
        "description": m.get("description").and_then(Value::as_str)
            .map(|t| truncate_chars(&html_to_markdown(t), 300)),
    });
    // Cross-link activities to their authoritative records.
    if modname.as_str() == Some("assign")
        && let Some(id) = m.get("id").and_then(Value::as_i64)
        && let Some(a) = assignments_by_cmid.get(&id)
    {
        entry["assignment_id"] = a.get("id").cloned().unwrap_or(Value::Null);
        entry["due"] = unix_to_iso(a.get("duedate").cloned().unwrap_or(Value::Null));
    }
    let contents: Vec<Value> = m
        .get("contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            let is_file = c.get("type").and_then(Value::as_str) == Some("file");
            let url = c.get("fileurl").cloned();
            if !is_file && url.is_none() {
                return None; // skip noise entries without a URL
            }
            Some(json!({
                "type": c.get("type").cloned().unwrap_or(Value::Null),
                "filename": c.get("filename").cloned().unwrap_or(Value::Null),
                "filesize": c.get("filesize").cloned().unwrap_or(Value::Null),
                "mimetype": c.get("mimetype").cloned().unwrap_or(Value::Null),
                "fileurl": url,
            }))
        })
        .collect();
    if !contents.is_empty() {
        entry["contents"] = Value::Array(contents);
    }
    entry
}

/// `get_course_overview` — merge five WS sources into one course map.
/// Returns `(markdown, structured)`.
pub async fn get_course_overview(
    client: &MoodleClient,
    course_id: i64,
) -> Result<(String, Value), MoodleError> {
    let contents_params = json!({ "courseid": course_id });
    let assign_params = json!({ "courseids": [course_id] });
    let quiz_params = json!({ "courseids": [course_id] });
    let forum_params = json!({ "courseids": [course_id] });
    let grade_params = json!({ "courseid": course_id });

    // Five sources, fetched concurrently.
    let (contents_r, assign_r, quiz_r, forum_r, grade_r) = tokio::join!(
        client.call::<Value>("core_course_get_contents", &contents_params),
        client.call::<Value>("mod_assign_get_assignments", &assign_params),
        client.call::<Value>("mod_quiz_get_quizzes_by_courses", &quiz_params),
        client.call::<Value>("mod_forum_get_forums_by_courses", &forum_params),
        client.call::<Value>("gradereport_user_get_grade_items", &grade_params),
    );

    // --- course meta (fullname from the assignments response; sections[0].name
    // is the first section, NOT the course name) ---
    let fullname = assign_r
        .as_ref()
        .ok()
        .and_then(|a| {
            crate::util::first_of(a, "courses")
                .and_then(|c| c.get("fullname"))
                .cloned()
        })
        .or_else(|| {
            grade_r.as_ref().ok().and_then(|g| {
                crate::util::first_of(g, "usergrades")
                    .and_then(|ug| ug.get("coursename"))
                    .cloned()
            })
        })
        .unwrap_or(json!(format!("course {course_id}")));

    // --- assignments ---
    let mut assignments: Vec<Value> = Vec::new();
    let mut assignments_by_cmid: std::collections::HashMap<i64, Value> = Default::default();
    if let Ok(a) = assign_r.as_ref() {
        for course in a
            .get("courses")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            for asg in course
                .get("assignments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if let Some(cmid) = asg.get("cmid").and_then(Value::as_i64) {
                    assignments_by_cmid.insert(cmid, asg.clone());
                }
                assignments.push(json!({
                    "assignment_id": asg.get("id").cloned().unwrap_or(Value::Null),
                    "cmid": asg.get("cmid").cloned().unwrap_or(Value::Null),
                    "name": asg.get("name").cloned().unwrap_or(Value::Null),
                    "due": unix_to_iso(asg.get("duedate").cloned().unwrap_or(Value::Null)),
                    "draft_required": asg.get("submissiondraft").cloned().unwrap_or(Value::Null),
                    "attachments": asg.get("introattachments").and_then(Value::as_array)
                        .cloned().unwrap_or_default().into_iter().map(|f| json!({
                            "filename": f.get("filename").cloned().unwrap_or(Value::Null),
                            "fileurl": f.get("fileurl").cloned().unwrap_or(Value::Null),
                        })).collect::<Vec<_>>(),
                }));
            }
        }
    }
    assignments.sort_by_key(|a| {
        a.get("due")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default()
    });

    // --- quizzes ---
    let quizzes_raw = quiz_r
        .as_ref()
        .ok()
        .and_then(|v| dig(v, "quizzes").cloned())
        .unwrap_or_else(|| json!([]));
    let quizzes: Vec<Value> = quizzes_raw
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|q| {
            json!({
                "quiz_id": q.get("id").cloned().unwrap_or(Value::Null),
                "cmid": q.get("cmid").cloned().unwrap_or(Value::Null),
                "name": q.get("name").cloned().unwrap_or(Value::Null),
                "opens": unix_to_iso(q.get("timeopen").cloned().unwrap_or(Value::Null)),
                "closes": unix_to_iso(q.get("timeclose").cloned().unwrap_or(Value::Null)),
            })
        })
        .collect();

    // --- forums ---
    let forums: Vec<Value> = forum_r
        .as_ref()
        .map(|v| v.as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            json!({
                "forum_id": f.get("id").cloned().unwrap_or(Value::Null),
                "name": f.get("name").cloned().unwrap_or(Value::Null),
                "type": f.get("type").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    // --- grades (graded items only) ---
    let (grades_section_md, grades_structured): (String, Value) = match grade_r {
        Ok(g) => {
            let items: Vec<Value> = crate::util::first_of(&g, "usergrades")
                .and_then(|ug| ug.get("gradeitems"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let graded: Vec<(String, String)> = items
                .iter()
                .filter_map(|it| {
                    let grade = it
                        .get("gradeformatted")
                        .filter(|v| !v.is_null())
                        .cloned()
                        .or_else(|| it.get("graderaw").cloned())?;
                    let grade_s = match &grade {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    if grade_s.is_empty() || grade_s == "-" {
                        return None; // not graded yet
                    }
                    let name = it
                        .get("itemname")
                        .filter(|v| !v.is_null())
                        .cloned()
                        .or_else(|| it.get("itemtype").cloned())
                        .unwrap_or(Value::Null);
                    Some((decode_entities(name.as_str().unwrap_or_default()), grade_s))
                })
                .collect();
            if graded.is_empty() {
                ("_No graded items yet._".into(), json!({"items": []}))
            } else {
                let md = graded
                    .iter()
                    .map(|(n, g)| format!("- {n}: {g}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                (md, json!({ "items": graded }))
            }
        }
        Err(e) => (
            format!("_grades: unavailable ({e})_"),
            json!({"unavailable": true}),
        ),
    };

    // --- sections (with cross-linked ids) ---
    let sections: Vec<Value> = contents_r
        .as_ref()
        .map(|v| v.as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            json!({
                "section_id": s.get("id").cloned().unwrap_or(Value::Null),
                "name": s.get("name").cloned().unwrap_or(Value::Null),
                "summary": s.get("summary").and_then(Value::as_str)
                    .map(|t| truncate_chars(&html_to_markdown(t), 300)),
                "modules": s.get("modules").and_then(Value::as_array)
                    .cloned().unwrap_or_default()
                    .into_iter()
                    .map(|m| shape_module(&m, &assignments_by_cmid))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let structured = json!({
        "course_id": course_id,
        "fullname": fullname,
        "assignments": assignments,
        "quizzes": quizzes,
        "forums": forums,
        "grades": grades_structured,
        "sections": sections,
    });

    // --- markdown rendering ---
    let mut md = String::new();
    md.push_str(&format!(
        "# {} (course_id: {course_id})\n\n",
        fullname.as_str().unwrap_or("?")
    ));

    md.push_str("## Assignments\n");
    if assignments.is_empty() {
        md.push_str("_none_\n");
    }
    for a in &assignments {
        md.push_str(&format!(
            "- {} — due: {} (assignment_id: {}, cmid: {})\n",
            a["name"].as_str().unwrap_or("?"),
            a["due"].as_str().unwrap_or("—"),
            a["assignment_id"],
            a["cmid"],
        ));
    }

    md.push_str("\n## Quizzes\n");
    if quizzes.is_empty() {
        md.push_str("_none_\n");
    }
    for q in &quizzes {
        md.push_str(&format!(
            "- {} — closes: {} (quiz_id: {})\n",
            q["name"].as_str().unwrap_or("?"),
            q["closes"].as_str().unwrap_or("—"),
            q["quiz_id"],
        ));
    }

    md.push_str("\n## Forums\n");
    if forums.is_empty() {
        md.push_str("_none_\n");
    }
    for f in &forums {
        md.push_str(&format!(
            "- {} (forum_id: {})\n",
            f["name"].as_str().unwrap_or("?"),
            f["forum_id"],
        ));
    }

    md.push_str("\n## Grades (graded items)\n");
    md.push_str(&grades_section_md);
    md.push('\n');

    md.push_str("\n## Sections\n");
    for s in &sections {
        md.push_str(&format!(
            "### {} (section_id: {})\n",
            s["name"].as_str().unwrap_or("?"),
            s["section_id"],
        ));
        for m in s["modules"].as_array().unwrap_or(&Vec::new()) {
            md.push_str(&format!(
                "- [{}] {} (module_id: {}{}\n",
                m["type"].as_str().unwrap_or("?"),
                m["name"].as_str().unwrap_or("?"),
                m["module_id"],
                m.get("assignment_id")
                    .map(|a| format!(", assignment_id: {a})"))
                    .unwrap_or_else(|| ")".into()),
            ));
            for c in m["contents"].as_array().unwrap_or(&Vec::new()) {
                md.push_str(&format!(
                    "    - file: {} ({} bytes) — {}\n",
                    c["filename"].as_str().unwrap_or("?"),
                    c["filesize"],
                    c["fileurl"].as_str().unwrap_or("no url"),
                ));
            }
        }
    }

    Ok((md, structured))
}
