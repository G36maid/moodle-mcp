use serde_json::Value;

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

/// Naively strip HTML tags and decode the most common entities.
/// Good enough to make Moodle's HTML blobs readable for an LLM.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // Treat <br>, </p>, <li>, <div> boundaries as newlines.
                let rest: String = chars.clone().take(8).collect();
                let lower = rest.to_lowercase();
                if lower.starts_with("br") || lower.starts_with("/p") || lower.starts_with("li>")
                    || lower.starts_with("/li") || lower.starts_with("div") || lower.starts_with("/div")
                    || lower.starts_with("p>") || lower.starts_with("tr>") || lower.starts_with("h1")
                    || lower.starts_with("h2") || lower.starts_with("h3")
                {
                    out.push('\n');
                }
                in_tag = true;
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    // Collapse >2 consecutive newlines.
    let mut collapsed = String::with_capacity(out.len());
    let mut newlines = 0;
    for c in out.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                collapsed.push(c);
            }
        } else {
            newlines = 0;
            collapsed.push(c);
        }
    }
    collapsed.trim().to_string()
}

/// Truncate a string to `max` characters, appending an ellipsis marker.
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
        && obj.get("warnings").and_then(Value::as_array).is_some_and(|a| a.is_empty()) {
            obj.remove("warnings");
        }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_html() {
        let html = "<p>Hello&nbsp;<strong>world</strong></p><br><p>Line2 &amp; more</p>";
        assert_eq!(strip_html(html), "Hello world\n\nLine2 & more");
    }

    #[test]
    fn truncates() {
        assert_eq!(truncate_chars("abc", 10), "abc");
        let t = truncate_chars("abcdef", 3);
        assert!(t.starts_with("abc"));
        assert!(t.contains("truncated"));
    }

    #[test]
    fn unix_iso() {
        assert_eq!(unix_to_iso(json!(0)), Value::Null);
        assert_eq!(unix_to_iso(json!(1_700_000_000)), json!("2023-11-14T22:13:20Z"));
    }

    #[test]
    fn digs() {
        let v = json!({"a": {"b": [1, 2]}});
        assert_eq!(dig(&v, "a.b"), Some(&json!([1, 2])));
    }
}
