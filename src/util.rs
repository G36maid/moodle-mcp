//! Shared helpers: HTML conversion, time formatting, truncation, JSON tools.

use std::sync::OnceLock;

use serde_json::Value;

/// Process-wide timezone used to render Moodle timestamps. Stays UTC until
/// [set_timezone] installs the resolved one at startup (the process serves
/// exactly one Moodle site, so a global is the pragmatic choice).
static TIMEZONE: OnceLock<chrono_tz::Tz> = OnceLock::new();

/// Install the timezone all timestamps render in (startup only; the first
/// call wins, later calls are no-ops).
pub fn set_timezone(tz: chrono_tz::Tz) {
    let _ = TIMEZONE.set(tz);
}

/// The active rendering timezone (UTC until startup resolution).
pub fn timezone() -> chrono_tz::Tz {
    *TIMEZONE.get().unwrap_or(&chrono_tz::Tz::UTC)
}

/// Parse an IANA timezone name (e.g. `Asia/Taipei`).
pub fn parse_timezone(name: &str) -> Result<chrono_tz::Tz, String> {
    name.trim()
        .parse()
        .map_err(|_| format!("'{name}' is not an IANA timezone name (e.g. Asia/Taipei)"))
}

/// Parse one Moodle timezone setting; `99` ("use the next level"), empty
/// values and legacy numeric offsets give `None`.
fn moodle_timezone(value: Option<&Value>) -> Option<chrono_tz::Tz> {
    parse_timezone(value?.as_str()?).ok()
}

/// The site's `forcetimezone` as a web-service token sees it.
#[derive(Debug, PartialEq)]
pub(crate) enum Forced {
    No,
    Zone(chrono_tz::Tz),
    /// Set, but not an IANA name (e.g. a legacy `8.0`).
    Unreadable,
}

fn mobile_setting<'a>(config: &'a Value, name: &str) -> Option<&'a Value> {
    config
        .get("settings")?
        .as_array()?
        .iter()
        .find(|s| s.get("name").and_then(Value::as_str) == Some(name))?
        .get("value")
}

/// The site's `forcetimezone` and default `timezone`, in that order, from
/// `tool_mobile_get_config`. Only Moodle 4.2+ returns them:
/// https://github.com/moodle/moodle/blob/v4.5.4/admin/tool/mobile/classes/api.php#L376-L379
pub(crate) fn timezones_from_mobile_config(config: &Value) -> (Forced, Option<chrono_tz::Tz>) {
    let forced = match mobile_setting(config, "forcetimezone") {
        None | Some(Value::Null) => Forced::No,
        Some(v) if v.as_str().map(str::trim) == Some("99") || v.as_f64() == Some(99.0) => {
            Forced::No
        }
        v => moodle_timezone(v).map_or(Forced::Unreadable, Forced::Zone),
    };
    (forced, moodle_timezone(mobile_setting(config, "timezone")))
}

/// The profile timezone from a `core_user_get_users_by_field` entry. Since
/// 4.0.8 and 4.1.3 `hiddenuserfields` can hide it, even from its owner:
/// https://github.com/moodle/moodle/blob/v4.5.4/user/lib.php#L467-L469
pub(crate) fn timezone_from_user(user: &Value) -> Option<chrono_tz::Tz> {
    moodle_timezone(user.get("timezone"))
}

/// Convert Moodle's HTML fragments (course descriptions, forum posts,
/// notification bodies…) to markdown.
///
/// Uses `htmd` (turndown.js-inspired; preserves tables, lists and links).
/// Falls back to a minimal internal cleaner if the converter rejects the
/// input. Empty input returns empty output.
pub fn html_to_markdown(html: &str) -> String {
    let trimmed = html.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match htmd::convert(trimmed) {
        // htmd output can retain (sometimes double-encoded) entities and turns
        // &nbsp; into U+00A0; normalize both away.
        Ok(md) => decode_entities(&md).replace('\u{00A0}', " "),
        Err(_) => strip_html_fallback(trimmed),
    }
}

/// Fallback when htmd fails: strip tags naively (tables degrade to lines).
fn strip_html_fallback(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                out.push(' ');
                in_tag = true;
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decode HTML entities. Runs up to 3 passes — Moodle fields are sometimes
/// double-encoded (`&amp;nbsp;`), and a single pass would leave residue.
pub fn decode_entities(s: &str) -> String {
    let mut cur = s.to_string();
    for _ in 0..3 {
        let next = decode_entities_once(&cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn decode_entities_once(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Convert a unix timestamp (seconds) to an ISO-8601 / RFC-3339 string in
/// `tz` (e.g. `2026-10-20T00:00:00+08:00`; UTC keeps the `Z` suffix).
pub fn unix_to_iso_with(tz: chrono_tz::Tz, secs: Value) -> Value {
    match secs.as_i64() {
        Some(0) | None => Value::Null,
        Some(s) => match chrono::DateTime::from_timestamp(s, 0) {
            Some(dt) => Value::String(
                dt.with_timezone(&tz)
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ),
            None => Value::String(s.to_string()),
        },
    }
}

/// Convert a unix timestamp (seconds) to an ISO-8601 / RFC-3339 string in
/// the active [timezone].
pub fn unix_to_iso(secs: Value) -> Value {
    unix_to_iso_with(timezone(), secs)
}

/// Current time as a unix timestamp in seconds.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A unique integer suitable for Moodle draft-area item ids.
///
/// Combines the wall clock with a process-wide atomic counter: the counter
/// guarantees uniqueness across concurrent calls (the old `nanos % 1_000`
/// scheme kept only 10 distinct values per second on Windows' 100 ns clock,
/// so parallel `fetch_file` calls could collide), and the clock keeps ids
/// distinct across seconds and far apart in value space. A collision would
/// need 1 000 000 calls within the same second.
pub fn unique_item_id() -> i64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    now_unix() * 1_000_000 + ((n % 1_000_000) as i64)
}

/// Extract the major version from a Moodle `release` string, e.g.
/// `"4.5.4+ (Build: 20250417)"` → `Some(4)`, `"3.7"` → `Some(3)`.
/// Returns `None` for anything unparseable.
pub fn release_major(release: &str) -> Option<u32> {
    let digits: String = release.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Truncate a string to `max` characters, appending a neutral marker.
/// Callers that have a dedicated "full content" tool add their own pointer.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}… [truncated]")
}

/// Server-provided error text reaches the terminal through error messages —
/// strip control characters (escape-sequence injection) and cap length.
pub fn echo_safe(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// The loopback carve-out for the plaintext-credential guard: local test
/// instances (the same exemption `MOODLE_ALLOW_HTTP` makes) plus
/// localhost-shaped names.
pub fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Fetch a value from nested JSON by a `a.b.c`-style path (objects only).
pub fn dig<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |acc, key| acc.get(key))
}

/// Return the first element of the array at `v[key]`, if present.
pub fn first_of<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).and_then(Value::as_array).and_then(|a| a.first())
}

/// First non-null string field among `keys` (explicit `null` counts as absent).
pub fn first_str<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| v.get(*k).and_then(Value::as_str))
}

/// Remove `warnings` arrays that are empty, to reduce output noise.
pub fn strip_empty_warnings(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut()
        && obj
            .get("warnings")
            .and_then(Value::as_array)
            .is_some_and(|a| a.is_empty())
    {
        obj.remove("warnings");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn htmd_converts_tables() {
        let md = html_to_markdown(
            "<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>",
        );
        assert!(md.contains('|'), "table should survive as markdown: {md}");
        assert!(md.contains("A"));
    }

    #[test]
    fn strips_html_fallback_decodes_entities() {
        assert_eq!(
            strip_html_fallback("Hello&nbsp;&amp;&nbsp;<b>world</b>"),
            "Hello & world"
        );
    }

    #[test]
    fn html_double_encoded_and_nbsp() {
        // Label fields can arrive double-encoded (&amp;nbsp;) and htmd
        // emits U+00A0 for &nbsp;.
        let md = html_to_markdown("Group 2 :&amp;nbsp;electricity");
        assert!(!md.contains("&nbsp;"), "{md}");
        assert!(!md.contains("&amp;"), "{md}");
        assert!(!md.contains('\u{00A0}'), "{md}");
    }

    #[test]
    fn html_empty_is_empty() {
        assert_eq!(html_to_markdown("   "), "");
    }

    #[test]
    fn truncates() {
        assert_eq!(truncate_chars("abc", 10), "abc");
        let t = truncate_chars("abcdef", 3);
        assert!(t.starts_with("abc"));
        assert!(t.ends_with("[truncated]"));
    }

    #[test]
    fn unix_iso_rendering() {
        // Global default is UTC until set_timezone runs (startup only).
        assert_eq!(unix_to_iso(json!(0)), Value::Null);
        assert_eq!(unix_to_iso(json!(null)), Value::Null);
        assert_eq!(
            unix_to_iso(json!(1_700_000_000)),
            json!("2023-11-14T22:13:20Z")
        );
        let taipei = parse_timezone("Asia/Taipei").unwrap();
        // UTC 2023-11-14T22:13:20Z → +08:00 crosses the date boundary — the
        // exact off-by-a-day case from issue #8.
        assert_eq!(
            unix_to_iso_with(taipei, json!(1_700_000_000)),
            json!("2023-11-15T06:13:20+08:00")
        );
        // Explicit UTC keeps the Z suffix; zero stays null in any zone.
        assert_eq!(
            unix_to_iso_with(chrono_tz::Tz::UTC, json!(1_700_000_000)),
            json!("2023-11-14T22:13:20Z")
        );
        assert_eq!(unix_to_iso_with(taipei, json!(0)), Value::Null);
        // Out-of-range seconds fall back to the raw number as a string.
        assert_eq!(
            unix_to_iso_with(taipei, json!(i64::MAX)),
            json!(i64::MAX.to_string())
        );
    }

    #[test]
    fn timezones_from_mobile_config_settings() {
        let taipei = parse_timezone("Asia/Taipei").unwrap();
        // forcetimezone 99 means nothing is forced.
        let config = json!({"settings": [
            {"name": "timezone", "value": "Asia/Taipei"},
            {"name": "forcetimezone", "value": "99"}
        ], "warnings": []});
        assert_eq!(
            timezones_from_mobile_config(&config),
            (Forced::No, Some(taipei))
        );
        let forced = json!({"settings": [
            {"name": "timezone", "value": "99"},
            {"name": "forcetimezone", "value": "Europe/Prague"}
        ]});
        assert_eq!(
            timezones_from_mobile_config(&forced),
            (Forced::Zone(parse_timezone("Europe/Prague").unwrap()), None)
        );
        let only_forced = |value: Value| {
            timezones_from_mobile_config(&json!({"settings": [
                {"name": "forcetimezone", "value": value}
            ]}))
            .0
        };
        for value in [json!(99), json!(null)] {
            assert_eq!(only_forced(value), Forced::No);
        }
        // A legacy offset or a bad name is still forced, just unreadable.
        for value in [json!("8.0"), json!("Mars/Olympus"), json!("")] {
            assert_eq!(only_forced(value), Forced::Unreadable);
        }
        // Before 4.2 the section is empty.
        assert_eq!(
            timezones_from_mobile_config(&json!({"settings": []})),
            (Forced::No, None)
        );
        assert_eq!(timezones_from_mobile_config(&json!({})), (Forced::No, None));
    }

    #[test]
    fn timezone_from_user_profile() {
        assert_eq!(
            timezone_from_user(&json!({"id": 5, "timezone": "America/New_York"})),
            Some(parse_timezone("America/New_York").unwrap())
        );
        // 99 (site default), a hidden field and a legacy offset give `None`.
        assert_eq!(
            timezone_from_user(&json!({"id": 5, "timezone": "99"})),
            None
        );
        assert_eq!(timezone_from_user(&json!({"id": 5})), None);
        assert_eq!(
            timezone_from_user(&json!({"id": 5, "timezone": "8.0"})),
            None
        );
        assert!(parse_timezone("Not/A_Zone").is_err());
    }

    #[test]
    fn digs() {
        let v = json!({"a": {"b": [1, 2]}});
        assert_eq!(dig(&v, "a.b"), Some(&json!([1, 2])));
    }

    #[test]
    fn parses_release_majors() {
        assert_eq!(release_major("4.5.4+ (Build: 20250417)"), Some(4));
        assert_eq!(release_major("3.7"), Some(3));
        assert_eq!(release_major("5.0.2 (Build: 20250609)"), Some(5));
        assert_eq!(release_major(""), None);
        assert_eq!(release_major("unknown"), None);
    }

    #[test]
    fn unique_item_ids_stay_unique_under_parallelism() {
        // Regression for the Windows clock-resolution collision: many
        // concurrent calls must never hand out the same id.
        let ids = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..8 {
                ids.lock().unwrap().reserve(500);
                s.spawn(|| {
                    let local: Vec<i64> = (0..500).map(|_| unique_item_id()).collect();
                    ids.lock().unwrap().extend(local);
                });
            }
        });
        let mut all = ids.into_inner().unwrap();
        let total = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), total, "parallel unique_item_id calls collided");
    }
}
