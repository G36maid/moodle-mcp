use base64::Engine as _;
use serde_json::{json, Value};

use crate::client::MoodleClient;
use crate::error::MoodleError;

const MAX_TEXT_BYTES: usize = 1_000_000; // 1 MB
/// Inline content must stay under the server's output cap (250k chars) with
/// headroom for the surrounding JSON — larger files must go through save_to.
const MAX_INLINE_CHARS: usize = 200_000;

/// Download a Moodle file. Uses the web-service token (injected into
/// `pluginfile.php` URLs when missing).
///
/// With `save_to`, the raw bytes are written to that local path and only
/// metadata is returned — binary payloads never enter the conversation.
/// Without it, small/decodable content is returned inline as text and other
/// content as base64.
pub async fn download_file(
    client: &MoodleClient,
    url: &str,
    save_to: Option<&str>,
) -> Result<Value, MoodleError> {
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

    if let Some(dest) = save_to {
        tokio::fs::write(dest, &bytes)
            .await
            .map_err(|e| MoodleError::Config(format!("failed to write {dest}: {e}")))?;
        return Ok(json!({
            "url": url,
            "status": status,
            "content_type": content_type,
            "size_bytes": size,
            "saved_to": dest,
        }));
    }

    // Prefer inline text for small, decodable payloads regardless of mime type;
    // otherwise base64-encode. Content that would overflow the output cap gets
    // a clean error pointing at save_to (never truncated JSON).
    let (encoding, content) = if size <= MAX_TEXT_BYTES {
        match std::str::from_utf8(&bytes) {
            Ok(text) => ("utf-8".to_string(), text.to_string()),
            Err(_) => (
                "base64".to_string(),
                base64::engine::general_purpose::STANDARD.encode(&bytes),
            ),
        }
    } else {
        (
            "base64".to_string(),
            base64::engine::general_purpose::STANDARD.encode(&bytes),
        )
    };
    if content.chars().count() > MAX_INLINE_CHARS {
        return Err(MoodleError::InvalidBody(format!(
            "file is {size} bytes ({} chars encoded) — too large to return inline; \
             call again with save_to to write it to a local path",
            content.chars().count()
        )));
    }

    Ok(json!({
        "url": url,
        "status": status,
        "content_type": content_type,
        "size_bytes": size,
        "encoding": encoding,
        "content": content
    }))
}

/// `webservice/upload.php` — upload bytes into the user's draft file area.
/// Returns the draft `item_id` referencing the uploaded file.
async fn upload_bytes(
    client: &MoodleClient,
    filename: &str,
    bytes: Vec<u8>,
    item_id: i64,
) -> Result<Value, MoodleError> {
    let v = client.upload_to_draft(filename, bytes, item_id).await?;
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
    }))
}

/// Upload base64-encoded content to the user's draft file area.
pub async fn upload_file_inline(
    client: &MoodleClient,
    filename: &str,
    content_base64: &str,
    item_id: i64,
) -> Result<Value, MoodleError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content_base64.trim())
        .map_err(|e| MoodleError::Config(format!("content is not valid base64: {e}")))?;
    upload_bytes(client, filename, bytes, item_id).await
}

/// Upload a local file (read from disk — binary data never enters the
/// conversation). `filename` defaults to the path's file name.
pub async fn upload_file_from_path(
    client: &MoodleClient,
    path: &str,
    filename: Option<&str>,
    item_id: i64,
) -> Result<Value, MoodleError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| MoodleError::Config(format!("failed to read {path}: {e}")))?;
    let name = filename
        .map(str::to_string)
        .unwrap_or_else(|| basename(path));
    upload_bytes(client, &name, bytes, item_id).await
}

fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
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

    #[test]
    fn extracts_basename() {
        assert_eq!(basename("/tmp/a/b/homework.pdf"), "homework.pdf");
        assert_eq!(basename("plain.txt"), "plain.txt");
    }
}
