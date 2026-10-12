use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::{dig, strip_html, truncate_chars, unix_to_iso};

/// `core_enrol_get_users_courses` — list courses a user is enrolled in.
pub async fn list_courses(
    client: &MoodleClient,
    userid: i64,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call("core_enrol_get_users_courses", &json!({ "userid": userid }))
        .await?;
    let courses: Vec<Value> = v
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            json!({
                "course_id": c.get("id").cloned().unwrap_or(Value::Null),
                "fullname": c.get("fullname").cloned().unwrap_or(Value::Null),
                "shortname": c.get("shortname").cloned().unwrap_or(Value::Null),
                "displayname": c.get("displayname").cloned().unwrap_or(Value::Null),
                "category": c.get("category").cloned().unwrap_or(Value::Null),
                "progress_percent": c.get("progress").and_then(Value::as_f64),
                "startdate": unix_to_iso(c.get("startdate").cloned().unwrap_or(Value::Null)),
                "enddate": unix_to_iso(c.get("enddate").cloned().unwrap_or(Value::Null)),
                "visible": c.get("visible").cloned().unwrap_or(Value::Null),
                "summary": c.get("summary").and_then(Value::as_str).map(|s| truncate_chars(&strip_html(s), 300)),
            })
        })
        .collect();
    Ok(json!({ "count": courses.len(), "courses": courses }))
}

/// `core_course_get_contents` — sections, modules and file resources of a course.
pub async fn get_course_contents(
    client: &MoodleClient,
    course_id: i64,
    section_id: Option<i64>,
    section_number: Option<i64>,
    exclude_modules: bool,
    exclude_contents: bool,
) -> Result<Value, MoodleError> {
    let mut options: Vec<Value> = Vec::new();
    if exclude_modules {
        options.push(json!({"name": "excludemodules", "value": "1"}));
    }
    if exclude_contents {
        options.push(json!({"name": "excludecontents", "value": "1"}));
    }
    if let Some(id) = section_id {
        options.push(json!({"name": "sectionid", "value": id.to_string()}));
    }
    if let Some(num) = section_number {
        options.push(json!({"name": "sectionnumber", "value": num.to_string()}));
    }
    let params = if options.is_empty() {
        json!({ "courseid": course_id })
    } else {
        json!({ "courseid": course_id, "options": options })
    };
    let v: Value = client.call("core_course_get_contents", &params).await?;

    let sections: Vec<Value> = v
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let modules: Vec<Value> = s
                .get("modules")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|m| {
                    let contents: Vec<Value> = m
                        .get("contents")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| {
                            json!({
                                "type": c.get("type").cloned().unwrap_or(Value::Null),
                                "filename": c.get("filename").cloned().unwrap_or(Value::Null),
                                "filesize": c.get("filesize").cloned().unwrap_or(Value::Null),
                                "mimetype": c.get("mimetype").cloned().unwrap_or(Value::Null),
                                "fileurl": c.get("fileurl").cloned().unwrap_or(Value::Null),
                                "timemodified": unix_to_iso(c.get("timemodified").cloned().unwrap_or(Value::Null)),
                                "content": c.get("content").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 1000)),
                            })
                        })
                        .collect();
                    json!({
                        "module_id": m.get("id").cloned().unwrap_or(Value::Null),
                        "name": m.get("name").cloned().unwrap_or(Value::Null),
                        "modname": m.get("modname").cloned().unwrap_or(Value::Null),
                        "url": m.get("url").cloned().unwrap_or(Value::Null),
                        "description": m.get("description").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 500)),
                        "contents_count": contents.len(),
                        "contents": contents,
                    })
                })
                .collect();
            json!({
                "section_id": s.get("id").cloned().unwrap_or(Value::Null),
                "name": s.get("name").cloned().unwrap_or(Value::Null),
                "summary": s.get("summary").and_then(Value::as_str).map(|t| truncate_chars(&strip_html(t), 400)),
                "modules_count": modules.len(),
                "modules": modules,
            })
        })
        .collect();
    Ok(json!({ "course_id": course_id, "sections_count": sections.len(), "sections": sections }))
}

/// `core_course_search_courses` — full-text search over the course catalogue.
pub async fn search_courses(
    client: &MoodleClient,
    search: &str,
    limit: u32,
) -> Result<Value, MoodleError> {
    let v: Value = client
        .call(
            "core_course_search_courses",
            &json!({
                "criterianame": "search",
                "criteriavalue": search,
                "limitnum": limit
            }),
        )
        .await?;
    let courses: Vec<Value> = dig(&v, "courses")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            json!({
                "course_id": c.get("id").cloned().unwrap_or(Value::Null),
                "fullname": c.get("fullname").cloned().unwrap_or(Value::Null),
                "shortname": c.get("shortname").cloned().unwrap_or(Value::Null),
                "category": c.get("categoryname").cloned().unwrap_or(Value::Null),
                "summary": c.get("summary").and_then(Value::as_str).map(|s| truncate_chars(&strip_html(s), 300)),
            })
        })
        .collect();
    Ok(json!({
        "total": dig(&v, "total").cloned().unwrap_or(json!(courses.len())),
        "courses": courses
    }))
}
