use base64::Engine as _;
use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;

const MAX_TEXT_BYTES: usize = 1_000_000; // 1 MB
const MAX_BINARY_BYTES: usize = 8_000_000; // 8 MB

/// Download a file from Moodle. Uses the web-service token (injected into
/// `pluginfile.php` URLs when missing). Text-like files are returned inline;
/// binary files are returned base64-encoded.
pub async fn download_file(client: &MoodleClient, url: &str) -> Result<Value, MoodleError> {
    let resp = client.download_file(url).await?;
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string();

    if !(200..300).contains(&status) {
        return Err(MoodleError::InvalidBody(format!(
            "file download returned HTTP {status}; the URL may be expired or the file inaccessible"
        )));
    }

    let bytes = resp.bytes().await.map_err(MoodleError::from)?;
    let size = bytes.len();

    // Prefer inline text for small, decodable payloads regardless of mime type;
    // otherwise base64-encode. Reject oversized binaries.
    let (encoding, content) = if size <= MAX_TEXT_BYTES {
        match std::str::from_utf8(&bytes) {
            Ok(text) => ("utf-8".to_string(), text.to_string()),
            Err(_) => (
                "base64".to_string(),
                base64::engine::general_purpose::STANDARD.encode(&bytes),
            ),
        }
    } else if size > MAX_BINARY_BYTES {
        return Err(MoodleError::InvalidBody(format!(
            "file too large to return inline ({size} bytes, limit {MAX_BINARY_BYTES}); \
             open the URL directly in a browser"
        )));
    } else {
        (
            "base64".to_string(),
            base64::engine::general_purpose::STANDARD.encode(&bytes),
        )
    };

    Ok(json!({
        "url": url,
        "status": status,
        "content_type": content_type,
        "size_bytes": size,
        "encoding": encoding,
        "content": content
    }))
}

/// `webservice/upload.php` — upload a file into the user's draft file area.
/// Returns the draft `item_id` used to attach the file to a submission.
pub async fn upload_draft_file(
    client: &MoodleClient,
    filename: &str,
    content_base64: &str,
    item_id: i64,
) -> Result<Value, MoodleError> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_base64.trim())
        .map_err(|e| MoodleError::Config(format!("content is not valid base64: {e}")))?;
    let v = client
        .upload_to_draft(filename, bytes, item_id)
        .await?;
    let files = v.as_array().cloned().unwrap_or_default();
    if files.is_empty() {
        return Err(MoodleError::InvalidBody(
            "upload.php returned no file records".into(),
        ));
    }
    let first = &files[0];
    Ok(json!({
        "uploaded": true,
        "item_id": first.get("itemid").and_then(Value::as_i64).unwrap_or(item_id),
        "filename": first.get("filename").cloned().unwrap_or(json!(filename)),
        "filesize": first.get("filesize").cloned().unwrap_or(Value::Null),
        "note": "Use item_id with submit_assignment_files to attach this file to an assignment."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_base64() {
        let bad = "not-base64!!!";
        let decoded = base64::engine::general_purpose::STANDARD.decode(bad.trim());
        assert!(decoded.is_err());
    }
}
