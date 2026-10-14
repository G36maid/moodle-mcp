//! Courses: the entry point (`list_courses`) and the course map
//! (`get_course_overview` — 5 WS sources merged into one markdown document).

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{decode_entities, dig, html_to_markdown, unix_to_iso};

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

    let needle = search
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
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
        "name": decode_entities(m.get("name").and_then(Value::as_str).unwrap_or_default()),
        // Descriptions carry the assignment specs; JSON is never truncated
        // (see output.rs) — a per-field cap here silently dropped most of a
        // real course page (#13).
        "description": m.get("description").and_then(Value::as_str)
            .map(html_to_markdown),
    });
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
    // The course page greys restricted items out with an explanation; without
    // that text a locked activity is indistinguishable from a usable one
    // (real case: an exam sign-up "available from 2027-01-11"). Only
    // deviations from "plainly usable" are emitted to keep listings quiet.
    if let Some(info) = m
        .get("availabilityinfo")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        entry["availability_info"] = Value::String(html_to_markdown(info));
    }
    if m.get("uservisible").and_then(Value::as_bool) == Some(false) {
        entry["uservisible"] = Value::Bool(false);
    }
    if m.get("visible").and_then(Value::as_bool) == Some(false) {
        entry["visible"] = Value::Bool(false);
    }
    entry
}

/// `get_course_overview` — merge five WS sources into one course map.
/// Returns `(markdown, structured)`.
pub async fn get_course_overview(
    client: &MoodleClient,
    course_id: i64,
    user_id: i64,
) -> Result<(String, Value), MoodleError> {
    let contents_params = json!({ "courseid": course_id });
    let assign_params = json!({ "courseids": [course_id] });
    let quiz_params = json!({ "courseids": [course_id] });
    let forum_params = json!({ "courseids": [course_id] });
    let grade_params = json!({ "courseid": course_id, "userid": user_id });

    let (contents_r, assign_r, quiz_r, forum_r, grade_r) = tokio::join!(
        client.call("core_course_get_contents", &contents_params),
        client.call("mod_assign_get_assignments", &assign_params),
        client.call("mod_quiz_get_quizzes_by_courses", &quiz_params),
        client.call("mod_forum_get_forums_by_courses", &forum_params),
        client.call("gradereport_user_get_grade_items", &grade_params),
    );

    // A failed contents fetch means the course is invalid/inaccessible —
    // surface it as an error instead of rendering an empty map.
    if let Err(e) = &contents_r {
        return Err(MoodleError::InvalidBody(e.to_tool_message()));
    }

    // fullname comes from the assignments response; sections[0].name is the
    // first section, NOT the course name.
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
        .map(|v| decode_entities(v.as_str().unwrap_or("?")))
        .unwrap_or(format!("course {course_id}"));

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
                    "name": decode_entities(asg.get("name").and_then(Value::as_str).unwrap_or_default()),
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
                "name": decode_entities(q.get("name").and_then(Value::as_str).unwrap_or_default()),
                "opens": unix_to_iso(q.get("timeopen").cloned().unwrap_or(Value::Null)),
                "closes": unix_to_iso(q.get("timeclose").cloned().unwrap_or(Value::Null)),
            })
        })
        .collect();

    let forums: Vec<Value> = forum_r
        .as_ref()
        .map(|v| v.as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
        .into_iter()
        .map(|f| {
            json!({
                "forum_id": f.get("id").cloned().unwrap_or(Value::Null),
                "name": decode_entities(f.get("name").and_then(Value::as_str).unwrap_or_default()),
                "type": f.get("type").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

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

    let sections: Vec<Value> = contents_r
        .as_ref()
        .map(|v| v.as_array().cloned().unwrap_or_default())
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let mut section = json!({
                "section_id": s.get("id").cloned().unwrap_or(Value::Null),
                "name": decode_entities(s.get("name").and_then(Value::as_str).unwrap_or_default()),
                // Full summary, same no-per-field-cap rule as descriptions:
                // teachers embed lecture links, instructions and notices here.
                "summary": s.get("summary").and_then(Value::as_str)
                    .map(html_to_markdown),
                "modules": s.get("modules").and_then(Value::as_array)
                    .cloned().unwrap_or_default()
                    .into_iter()
                    .map(|m| shape_module(&m, &assignments_by_cmid))
                    .collect::<Vec<_>>(),
            });
            // Same quiet-unless-deviating rule as modules: restricted/hidden
            // sections exist for teachers and in greyed-out student views.
            if let Some(info) = s
                .get("availabilityinfo")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                section["availability_info"] = Value::String(html_to_markdown(info));
            }
            if s.get("visible").and_then(Value::as_bool) == Some(false) {
                section["visible"] = Value::Bool(false);
            }
            if s.get("uservisible").and_then(Value::as_bool) == Some(false) {
                section["uservisible"] = Value::Bool(false);
            }
            if s.get("hiddenbynumsections").and_then(Value::as_i64) == Some(1) {
                section["hidden_beyond_course_end"] = Value::Bool(true);
            }
            section
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

    let mut md = String::new();
    md.push_str(&format!("# {fullname} (course_id: {course_id})\n\n"));

    md.push_str("\n## Sections\n");
    for s in &sections {
        md.push_str(&format!(
            "### {} (section_id: {})\n",
            s["name"].as_str().unwrap_or("?"),
            s["section_id"],
        ));
        if let Some(info) = s["availability_info"].as_str() {
            md.push_str(&format!("\nRestricted: {info}\n"));
        }
        if let Some(summary) = s["summary"]
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            md.push('\n');
            md.push_str(&demote_headings(summary, 4));
            md.push_str("\n\n");
        }
        for m in s["modules"].as_array().unwrap_or(&Vec::new()) {
            let mut marks: Vec<String> = Vec::new();
            if let Some(info) = m["availability_info"].as_str() {
                marks.push(format!("restricted: {info}"));
            }
            if m["visible"].as_bool() == Some(false) {
                marks.push("hidden".into());
            }
            if m["uservisible"].as_bool() == Some(false) {
                marks.push("unavailable".into());
            }
            let marks_suffix = if marks.is_empty() {
                String::new()
            } else {
                format!(" — {}", marks.join("; "))
            };
            md.push_str(&format!(
                "- [{}] {} (module_id: {}{}\n",
                m["type"].as_str().unwrap_or("?"),
                m["name"].as_str().unwrap_or("?"),
                m["module_id"],
                m.get("assignment_id")
                    .map(|a| format!(", assignment_id: {a})"))
                    .unwrap_or_else(|| ")".into())
                    + &marks_suffix,
            ));
            if let Some(desc) = m["description"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                // Indented continuation of the bullet item: keeps paragraphs,
                // nested lists and the file lines below inside this module.
                md.push('\n');
                md.push_str(&indent_block(&headings_to_bold(desc), 2));
                md.push_str("\n\n");
            }
            for c in m["contents"].as_array().unwrap_or(&Vec::new()) {
                let kind = if c["type"].as_str() == Some("file") {
                    "file"
                } else {
                    "link"
                };
                let size = c["filesize"]
                    .as_i64()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "?".into());
                md.push_str(&format!(
                    "    - {kind}: {} ({size} bytes) — {}\n",
                    c["filename"].as_str().unwrap_or("?"),
                    c["fileurl"].as_str().unwrap_or("no url"),
                ));
            }
        }
    }

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

    Ok((md, structured))
}

// Moodle injects summary/description HTML into the page unescaped — `noclean`
// passthrough on 3.x, `{{{summarytext}}}` on 5.x — so the browser is the
// renderer and the fidelity bar: text the browser shows must appear here, and
// markup that renders as nothing (empty heading shells) must not. Converted
// ATX headings get demoted under the `###` section heading in section blocks;
// inside a list item (module description) they flatten to bold.
fn demote_headings(md: &str, min_level: usize) -> String {
    md.lines()
        .filter_map(|line| match heading_text(line) {
            Some(text) if text.is_empty() => None,
            Some(text) => {
                let src_level = line.bytes().take_while(|&b| b == b'#').count();
                let level = (src_level + 2).clamp(min_level, 6);
                Some(format!("{} {}", "#".repeat(level), text))
            }
            None => Some(line.to_string()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn headings_to_bold(md: &str) -> String {
    md.lines()
        .filter_map(|line| match heading_text(line) {
            Some(text) if text.is_empty() => None,
            // htmd renders heading content bold as-is (`### **title**`) or
            // partially bold (`### **Do not** do x`); any `**` inside means
            // the author's emphasis is already there — re-wrapping would
            // only make four-star soup. Literal `*` arrives escaped as
            // `\*`, so `**` here is always real emphasis.
            Some(text) if text.contains("**") => Some(text),
            Some(text) => Some(format!("**{text}**")),
            None => Some(line.to_string()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// If `line` is an ATX heading, its text with any further leading hash groups
/// stripped — htmd merges adjacent empty+real editor headings into lines like
/// `#### #### Title`, and empty shells survive as bare `##`. `None` for
/// non-heading lines (`#tag` included).
fn heading_text(line: &str) -> Option<String> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let mut text = rest.trim();
    while let Some(stripped) = text.strip_prefix('#') {
        text = stripped.trim_start();
        if text.is_empty() {
            break;
        }
    }
    Some(text.to_string())
}

fn indent_block(md: &str, width: usize) -> String {
    let pad = " ".repeat(width);
    md.lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demote_nests_under_section_heading() {
        // Summaries sit under `###` in the document; nothing they carry may
        // claim a higher outline level.
        assert_eq!(demote_headings("## H\n\npara", 4), "#### H\n\npara");
        assert_eq!(demote_headings("### H", 4), "##### H");
        assert_eq!(demote_headings("# H", 4), "#### H");
    }

    #[test]
    fn demote_caps_at_six_and_leaves_plain_text() {
        assert_eq!(demote_headings("###### H", 4), "###### H");
        // 7+ hashes and `#tag` without a space are not headings in CommonMark.
        assert_eq!(demote_headings("####### H", 4), "####### H");
        assert_eq!(demote_headings("#not-a-heading", 4), "#not-a-heading");
    }

    #[test]
    fn descriptions_flatten_headings_to_bold() {
        assert_eq!(headings_to_bold("### Title\nbody"), "**Title**\nbody");
        // Empty headings are editor noise (`<h3><br></h3>`) — drop the line.
        assert_eq!(headings_to_bold("### \nbody"), "body");
    }

    #[test]
    fn merged_and_empty_heading_shells_collapse_to_visible_text() {
        // Moodle's editor leaves empty `<h4>` shells next to real ones; htmd
        // merges them into doubled hashes. The page only ever shows the text.
        assert_eq!(
            headings_to_bold("#### #### TCP variants"),
            "**TCP variants**"
        );
        assert_eq!(headings_to_bold("##\nbody"), "body");
        assert_eq!(
            demote_headings("#### #### TCP variants", 4),
            "###### TCP variants"
        );
    }

    #[test]
    fn already_bold_headings_are_not_rewrapped() {
        // htmd emits `### **title**` for `<h3><b>title</b></h3>`; partial
        // bold in the heading content stays as authored too.
        assert_eq!(headings_to_bold("### **Already Bold**"), "**Already Bold**");
        assert_eq!(
            headings_to_bold("### **Do not** summarize the papers"),
            "**Do not** summarize the papers"
        );
    }

    #[test]
    fn indent_keeps_blank_lines_blank() {
        assert_eq!(indent_block("a\n\nb", 2), "  a\n\n  b");
    }
}
