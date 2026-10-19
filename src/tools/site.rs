//! Site identity and capability discovery.
//!
//! The raw `site_info` payload is fetched once (by `tools::MoodleMcp`) and
//! cached; these helpers shape what tools expose from it. [`detect_timezone`]
//! runs before that cache exists, so it calls Moodle itself.

use std::time::Duration;

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::{MoodleError, scrub_token};
use crate::util::{Forced, timezone_from_user, timezones_from_mobile_config};

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

/// `list_ws_functions` — filter the cached allowed-function list by a
/// case-insensitive substring. Errors when nothing matches, with usage hint.
pub fn list_ws_functions(allowed: &[String], query: &str) -> Result<Value, MoodleError> {
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

/// Detect the timezone Moodle uses for this user, with its source. Like
/// `core_date::get_user_timezone()`, a forced site zone beats the profile,
/// which beats the site default:
/// https://github.com/moodle/moodle/blob/v4.5.4/lib/classes/date.php#L237-L273
///
/// `budget` covers the whole lookup, since startup waits for it.
pub async fn detect_timezone(
    client: &MoodleClient,
    budget: Duration,
) -> Option<(chrono_tz::Tz, &'static str)> {
    // Instant + Duration panics on overflow, and the budget may be set huge.
    let deadline = tokio::time::Instant::now() + budget.min(Duration::from_secs(365 * 24 * 3600));
    let config = tokio::time::timeout_at(
        deadline,
        client.call(
            "tool_mobile_get_config",
            &json!({ "section": "locationsettings" }),
        ),
    )
    .await;
    let (forced, site_default) = match config {
        Ok(Ok(config)) => timezones_from_mobile_config(&config),
        // No answer, not web-service JSON (e.g. an HTML error page), or a
        // proxy/gateway status (429/5xx — the lookup would hammer a site
        // that is already telling us to back off): the profile calls would
        // fail the same way.
        Ok(Err(
            e @ (MoodleError::Http(_)
            | MoodleError::NonJsonResponse(_)
            | MoodleError::HttpStatus { .. }
            | MoodleError::HttpCooldown { .. }),
        )) => {
            let error = scrub_token(&e.to_string());
            tracing::info!(%error, "timezone lookup got no web-service answer");
            return None;
        }
        // Moodle answered but refused this function, e.g. one missing from
        // the token's service; the profile lookup may still work.
        Ok(Err(e)) => {
            tracing::debug!(
                error = %e,
                "tool_mobile_get_config unavailable for the timezone lookup"
            );
            (Forced::No, None)
        }
        Err(_) => {
            tracing::info!("timezone lookup timed out");
            return None;
        }
    };
    // A forced zone keeps the profile out even when it cannot be read.
    // Moodle then uses the site default for an invalid name, but maps a
    // legacy offset such as 8.0 to a fixed zone, which is not done here:
    // https://github.com/moodle/moodle/blob/v4.5.4/lib/classes/date.php#L137-L163
    match forced {
        Forced::Zone(tz) => return Some((tz, "forced site zone")),
        Forced::Unreadable => return site_default.map(|tz| (tz, "site default")),
        Forced::No => {}
    }
    match tokio::time::timeout_at(deadline, profile_timezone(client)).await {
        Ok(Some(tz)) => return Some((tz, "user profile")),
        Ok(None) => {}
        Err(_) => tracing::info!("profile timezone lookup timed out"),
    }
    site_default.map(|tz| (tz, "site default"))
}

/// The token owner's profile timezone, when the site shows it. It lives in
/// `user.timezone`, and the preference API reads only `user_preferences`:
/// https://github.com/moodle/moodle/blob/v4.5.4/lib/moodlelib.php#L1437
async fn profile_timezone(client: &MoodleClient) -> Option<chrono_tz::Tz> {
    let failed = |e: MoodleError| {
        let error = scrub_token(&e.to_string());
        tracing::debug!(%error, "profile timezone lookup failed");
    };
    let site = client
        .call("core_webservice_get_site_info", &json!({}))
        .await
        .map_err(failed)
        .ok()?;
    let userid = site.get("userid").and_then(Value::as_i64)?;
    let users = client
        .call(
            "core_user_get_users_by_field",
            &json!({ "field": "id", "values": [userid] }),
        )
        .await
        .map_err(failed)
        .ok()?;
    users.as_array()?.first().and_then(timezone_from_user)
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
        let out = list_ws_functions(&allowed, "forum").unwrap();
        assert_eq!(out["count"], json!(2));

        assert!(list_ws_functions(&allowed, "wiki").is_err());
        assert!(list_ws_functions(&allowed, "").is_err());
    }
}
