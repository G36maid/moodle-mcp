//! Output helpers: the dual-channel tool-result policy.
//!
//! Every tool returns `CallToolResult::structured(json)` — `text` is the
//! compact JSON string, `structuredContent` is the same JSON object (for
//! codemode scripts). Document tools (`get_course_overview`,
//! `get_assignment`) instead put a **markdown document** in `text` and keep
//! machine-readable JSON in `structuredContent`.
//!
//! Nulls are stripped recursively (Moodle 3.7 omits many fields); optional
//! hints are attached when the agent omits filter parameters; only the
//! markdown of document tools is ever truncated (plain text — never broken
//! JSON).

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Map, Value, json};

/// Hard cap for markdown documents (chars). Beyond this the text is truncated
/// with a marker; `structuredContent` is never truncated.
pub const MAX_DOC_CHARS: usize = 250_000;

/// Remove all `Value::Null` values from an object tree (recursively).
pub fn strip_nulls(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (k, val) in map {
                if val.is_null() {
                    continue;
                }
                out.insert(k.clone(), strip_nulls(val));
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let out: Vec<Value> = items
                .iter()
                .map(strip_nulls)
                .filter(|v| !v.is_null())
                .collect();
            Value::Array(out)
        }
        other => other.clone(),
    }
}

/// Attach hint lines to a JSON object (top-level `hints` array).
/// No-op for non-objects.
fn with_hints(mut v: Value, hints: &[String]) -> Value {
    if !hints.is_empty()
        && let Some(obj) = v.as_object_mut()
    {
        obj.insert(
            "hints".into(),
            Value::Array(hints.iter().map(|h| Value::String(h.clone())).collect()),
        );
    }
    v
}

/// Standard tool result: JSON in both channels (nulls stripped).
/// MCP requires `structuredContent` to be an object — non-object results
/// (raw arrays/scalars, e.g. from the escape hatch) are wrapped in
/// `{"result": …}`.
pub fn finish_json(v: Value) -> Result<CallToolResult, rmcp::ErrorData> {
    let v = strip_nulls(&v);
    let v = if v.is_object() {
        v
    } else {
        json!({ "result": v })
    };
    Ok(CallToolResult::structured(v))
}

/// Standard tool result with hint lines attached (top-level `hints` array).
pub fn finish_json_with_hints(
    v: Value,
    hints: &[String],
) -> Result<CallToolResult, rmcp::ErrorData> {
    let v = with_hints(strip_nulls(&v), hints);
    Ok(CallToolResult::structured(v))
}

/// Document-tool result: markdown in `text` (head-truncated), JSON in
/// `structuredContent`.
pub fn finish_doc(markdown: String, structured: Value) -> Result<CallToolResult, rmcp::ErrorData> {
    let total = markdown.chars().count();
    let text = if total > MAX_DOC_CHARS {
        let head: String = markdown.chars().take(MAX_DOC_CHARS).collect();
        format!(
            "{head}\n\n…[truncated: showing {MAX_DOC_CHARS} of {total} chars — narrow the query or use file tools to save the full content]"
        )
    } else {
        markdown
    };
    let mut result = CallToolResult::structured(strip_nulls(&structured));
    result.content = vec![ContentBlock::text(text)];
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_nulls_recursively() {
        let v = json!({"a": 1, "b": null, "c": {"d": null, "e": [1, null, {"f": null}]}});
        let out = strip_nulls(&v);
        assert_eq!(out, json!({"a": 1, "c": {"e": [1, {}]}}));
    }

    #[test]
    fn attaches_hints_only_when_present() {
        let out = finish_json_with_hints(json!({"a": 1}), &["hint one".into()]).unwrap();
        let sc = out.structured_content.unwrap();
        assert_eq!(sc["hints"][0], json!("hint one"));

        let out = finish_json(json!({"a": 1})).unwrap();
        assert!(out.structured_content.unwrap().get("hints").is_none());
    }

    #[test]
    fn non_object_results_are_wrapped_for_mcp() {
        // MCP requires structuredContent to be an object. Raw arrays/scalars
        // from the escape hatch (e.g. userid=-1 returns []) must be
        // wrapped in {"result": …}, or clients reject the response with
        // "Invalid MCP tools/call structured content".
        let out = finish_json(json!([1, 2])).unwrap();
        let sc = out.structured_content.unwrap();
        assert_eq!(sc["result"], json!([1, 2]));

        let out = finish_json(json!("plain")).unwrap();
        assert_eq!(out.structured_content.unwrap()["result"], json!("plain"));
    }

    #[test]
    fn doc_truncation_keeps_json_valid() {
        let md = "x".repeat(300_000);
        let out = finish_doc(md, json!({"k": 1})).unwrap();
        let text = out.content[0].as_text().unwrap().text.clone();
        assert!(text.len() < 260_000);
        assert!(text.contains("[truncated"));
        assert_eq!(out.structured_content.unwrap()["k"], json!(1));
    }
}
