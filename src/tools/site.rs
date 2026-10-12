//! Site identity and capability discovery.
//!
//! The raw `site_info` payload is fetched once (by `tools::MoodleMcp`) and
//! cached; these helpers shape what tools expose from it.

use serde_json::{Value, json};

use crate::error::MoodleError;

/// Shape the cached `core_webservice_get_site_info` payload into the
/// `whoami` output. Never fails — missing fields are simply omitted
/// downstream (nulls are stripped).
pub fn whoami_shape(site: &Value) -> Value {
    json!({
        "user_id": site.get("userid").cloned().unwrap_or(Value::Null),
        "username": site.get("username").cloned().unwrap_or(Value::Null),
        "fullname": site.get("fullname").cloned().unwrap_or(Value::Null),
        "lang": site.get("lang").cloned().unwrap_or(Value::Null),
        "site_name": site.get("sitename").cloned().unwrap_or(Value::Null),
        "site_url": site.get("siteurl").cloned().unwrap_or(Value::Null),
        "moodle_release": site.get("release").cloned().unwrap_or(Value::Null),
        "functions_count": site.get("functions").and_then(Value::as_array)
            .map(|a| json!(a.len())).unwrap_or(Value::Null),
    })
}

/// `list_functions` — filter the cached allowed-function list by a
/// case-insensitive substring. Errors when nothing matches, with usage hint.
pub fn list_functions(allowed: &[String], query: &str) -> Result<Value, MoodleError> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Err(MoodleError::Config(format!(
            "provide a search keyword to filter the {} available functions \
             (e.g. 'forum', 'assign', 'quiz')",
            allowed.len()
        )));
    }
    let matches: Vec<&str> = allowed
        .iter()
        .filter(|f| f.to_lowercase().contains(&q))
        .map(String::as_str)
        .collect();
    if matches.is_empty() {
        return Err(MoodleError::Config(format!(
            "no available function matches '{query}' ({} functions in total)",
            allowed.len()
        )));
    }
    Ok(json!({
        "query": query,
        "count": matches.len(),
        "functions": matches,
        "note": "These can be called via call_ws_function."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_functions() {
        let allowed: Vec<String> = vec![
            "mod_forum_get_discussions".into(),
            "mod_forum_add_discussion".into(),
            "core_grades_get_grades".into(),
        ];
        let out = list_functions(&allowed, "forum").unwrap();
        assert_eq!(out["count"], json!(2));

        assert!(list_functions(&allowed, "wiki").is_err());
        assert!(list_functions(&allowed, "").is_err());
    }
}
