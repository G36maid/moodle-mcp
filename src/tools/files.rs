//! File tools: fetch (read documents as markdown), download (bytes to disk),
//! upload (disk to draft area).

use serde_json::{Value, json};

use crate::client::MoodleClient;
use crate::error::MoodleError;
use crate::util::truncate_chars;

/// Inline cap for `fetch_file`: aligned with pi-web-access's
/// MAX_INLINE_FILE_CHARS (100k chars). Beyond this, head-truncate with a
/// marker and point at `download_file`.
const MAX_FETCH_CHARS: usize = 100_000;
/// Input cap for `fetch_file` (declared Content-Length must not exceed this).
const MAX_FETCH_INPUT_BYTES: u64 = 100_000_000; // 100 MB
/// Hard cap for streamed downloads (protects memory and disk).
const MAX_DOWNLOAD_BYTES: u64 = 500_000_000; // 500 MB
/// Fetch a Moodle file and return its content as **markdown** for the agent
/// to read. PDF/DOCX/PPTX/XLSX/ODT/RTF/EPUB/CSV are converted with anydoc;
/// plain text passes through; anything else (zip, images…) cannot be
/// converted — download it instead. The web-service token is injected into
/// `pluginfile.php` URLs automatically.
pub async fn fetch_file(client: &MoodleClient, url: &str) -> Result<Value, MoodleError> {
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
            "file fetch returned HTTP {status}; the URL may be expired or the file inaccessible"
        )));
    }
    // Size guard BEFORE buffering: reject anything the declared size says is
    // too big. Chunked/unknown lengths are re-checked by the inline cap below.
    if let Some(len) = resp.content_length()
        && len > MAX_FETCH_INPUT_BYTES
    {
        return Err(MoodleError::InvalidBody(format!(
            "file is {len} bytes — too large to fetch inline (limit {MAX_FETCH_INPUT_BYTES}); \
                 use download_file to save it to disk instead"
        )));
    }
    let bytes = resp.bytes().await.map_err(MoodleError::from)?;
    let size = bytes.len();

    // Documents → markdown via anydoc. Errors are transparent (encrypted /
    // needs OCR / unsupported), telling the agent to download instead.
    let convertable = [
        "pdf", "doc", "docx", "docm", "odt", "rtf", "epub", "ppt", "pps", "pot", "pptx", "pptm",
        "ppsx", "ppsm", "odp", "xls", "xlsx", "xlsm", "xlsb", "ods", "csv",
    ];
    let ext = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_lowercase();

    if convertable.contains(&ext.as_str()) {
        let tmp = format!(
            "{}/moodle-fetch-{}.{}",
            std::env::temp_dir().display(),
            crate::util::unique_item_id(),
            ext
        );
        std::fs::write(&tmp, &bytes)
            .map_err(|e| MoodleError::Config(format!("failed to buffer download: {e}")))?;
        let result = anydoc::to_markdown(&tmp);
        let _ = std::fs::remove_file(&tmp);
        match result {
            Ok(md) => {
                let total = md.chars().count();
                let text = if total > MAX_FETCH_CHARS {
                    let head: String = md.chars().take(MAX_FETCH_CHARS).collect();
                    format!(
                        "{head}\n\n…[truncated: {MAX_FETCH_CHARS} of {total} chars — use download_file to save the full document]"
                    )
                } else {
                    md
                };
                return Ok(json!({
                    "url": url,
                    "content_type": content_type,
                    "size_bytes": size,
                    "format": ext,
                    "chars": text.chars().count(),
                    "markdown": text,
                }));
            }
            Err(e) => {
                return Err(MoodleError::InvalidBody(format!(
                    "document conversion failed ({e}) — use download_file to save the original"
                )));
            }
        }
    }

    // Plain text (or decodable small payloads) → inline.
    if size <= 1_000_000
        && let Ok(text) = std::str::from_utf8(&bytes)
    {
        let text = truncate_chars(text, MAX_FETCH_CHARS);
        return Ok(json!({
            "url": url,
            "content_type": content_type,
            "size_bytes": size,
            "encoding": "utf-8",
            "content": text,
        }));
    }

    Err(MoodleError::InvalidBody(format!(
        "{content_type} content ({size} bytes) is not a convertible document — \
         use download_file to save the raw bytes to disk"
    )))
}

/// `download_file` — write a Moodle file to a local path. Uses the
/// web-service token (injected into `pluginfile.php` URLs when missing).
/// Returns metadata only; binary payloads never enter the conversation.
pub async fn download_file(
    client: &MoodleClient,
    url: &str,
    save_to: &str,
) -> Result<Value, MoodleError> {
    let mut resp = client.download_file(url).await?;
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
            "download returned HTTP {status}; the URL may be expired or the file inaccessible"
        )));
    }
    // Stream to disk in chunks (no full-body buffering) with a hard cap;
    // a mid-stream overage aborts and removes the partial file.
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(save_to)
        .await
        .map_err(|e| MoodleError::Config(format!("failed to create {save_to}: {e}")))?;
    let mut downloaded: u64 = 0;
    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = file.shutdown().await;
                let _ = tokio::fs::remove_file(save_to).await;
                return Err(MoodleError::Config(format!(
                    "download aborted after {downloaded} bytes: {e} (partial file removed)"
                )));
            }
        };
        downloaded += chunk.len() as u64;
        if downloaded > MAX_DOWNLOAD_BYTES {
            let _ = file.shutdown().await;
            let _ = tokio::fs::remove_file(save_to).await;
            return Err(MoodleError::Config(format!(
                "download exceeded the {MAX_DOWNLOAD_BYTES}-byte limit — aborted, partial file removed"
            )));
        }
        file.write_all(&chunk)
            .await
            .map_err(|e| MoodleError::Config(format!("failed to write {save_to}: {e}")))?;
    }
    file.flush()
        .await
        .map_err(|e| MoodleError::Config(format!("failed to flush {save_to}: {e}")))?;
    let size = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    Ok(json!({
        "url": url,
        "status": status,
        "content_type": content_type,
        "size_bytes": size,
        "saved_to": save_to,
    }))
}

/// Upload bytes into the user's draft file area. Returns the draft `item_id`
/// referencing the uploaded file.
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
