//! The escape hatch: call ANY Moodle WS function the token allows, plus
//! discovery (`list_functions`) and friendly suggestions on misses.

use serde_json::Value;

use crate::client::MoodleClient;
use crate::error::MoodleError;

/// Escape hatch: call any Moodle WS function by name with free-form JSON
/// parameters (nested objects/arrays are flattened to PHP bracket notation).
///
/// `allowed` is the token's enabled functions (from the cached site info);
/// the call is rejected client-side when the function is not in it — with a
/// fuzzy "did you mean" suggestion when a close name exists.
pub async fn call_ws_function(
    client: &MoodleClient,
    wsfunction: &str,
    params: &Value,
    allowed: &[String],
) -> Result<Value, MoodleError> {
    if !allowed.is_empty() && !allowed.iter().any(|f| f == wsfunction) {
        return Err(MoodleError::Config(format!(
            "function '{wsfunction}' is not enabled for this token. {}",
            suggest_functions(wsfunction, allowed)
        )));
    }
    client.call(wsfunction, params).await
}

/// Suggest close function names when an exact match is missing: closest
/// matches first, then the total count.
pub fn suggest_functions(requested: &str, allowed: &[String]) -> String {
    let req = requested.to_lowercase();
    let close: Vec<&str> = allowed
        .iter()
        .filter(|f| {
            let f = f.to_lowercase();
            f.contains(&req) || req.contains(&f) || levenshtein_limited(&req, &f, 6)
        })
        .take(3)
        .map(String::as_str)
        .collect();
    if close.is_empty() {
        format!(
            "{} functions are available; use list_functions with a keyword to explore them.",
            allowed.len()
        )
    } else {
        format!("Did you mean: {}?", close.join(", "))
    }
}

/// Bounded Levenshtein distance: returns true when the distance is ≤ max.
/// (Function names are short — plain full DP, no pruning.)
fn levenshtein_limited(a: &str, b: &str, max: usize) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return false;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()] <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_close_matches() {
        let allowed: Vec<String> = vec![
            "mod_forum_get_forum_discussions".into(),
            "mod_forum_get_forum_discussions_paginated".into(),
            "core_enrol_get_users_courses".into(),
        ];
        let msg = suggest_functions("mod_forum_get_discussions", &allowed);
        assert!(msg.contains("Did you mean"), "{msg}");
        assert!(msg.contains("mod_forum_get_forum_discussions"));
    }

    #[test]
    fn falls_back_to_count() {
        let msg = suggest_functions("zzz", &["core_enrol_get_users_courses".into()]);
        assert!(msg.contains("functions are available"), "{msg}");
    }

    #[test]
    fn levenshtein_bounded() {
        assert!(levenshtein_limited("abc", "axc", 1));
        assert!(!levenshtein_limited("abc", "xyz", 1));
    }
}
