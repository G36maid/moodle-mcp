//! Shared helpers: HTML conversion, time formatting, truncation, JSON tools.

use serde_json::Value;

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

/// Convert a unix timestamp (seconds) to an ISO-8601 / RFC-3339 string.
pub fn unix_to_iso(secs: Value) -> Value {
    match secs.as_i64() {
        Some(0) | None => Value::Null,
        Some(s) => match chrono::DateTime::from_timestamp(s, 0) {
            Some(dt) => Value::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            None => Value::String(s.to_string()),
        },
    }
}

/// Current time as a unix timestamp in seconds.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A unique integer suitable for Moodle draft-area item ids.
pub fn unique_item_id() -> i64 {
    let nanos = chrono::Utc::now().timestamp_subsec_nanos() as i64;
    now_unix() * 1_000 + (nanos % 1_000)
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
        // 3.7 live finding: label fields are double-encoded (&amp;nbsp;)
        // and htmd emits U+00A0 for &nbsp;.
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
    fn unix_iso() {
        assert_eq!(unix_to_iso(json!(0)), Value::Null);
        assert_eq!(
            unix_to_iso(json!(1_700_000_000)),
            json!("2023-11-14T22:13:20Z")
        );
    }

    #[test]
    fn digs() {
        let v = json!({"a": {"b": [1, 2]}});
        assert_eq!(dig(&v, "a.b"), Some(&json!([1, 2])));
    }
}
