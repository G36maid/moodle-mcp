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
/// Inline cap for plain-text / HTML payloads (same as the legacy text path).
const MAX_TEXT_BYTES: usize = 1_000_000; // 1 MB
/// Upload cap: Moodle sites cap uploads far below this (default ~8–50 MB);
/// the check avoids buffering an absurd file into RAM before the server
/// would reject it anyway.
const MAX_UPLOAD_BYTES: u64 = 100_000_000; // 100 MB
/// Upper bound for buffering a JSON response in download_file to decide
/// whether it is a Moodle error payload. Legitimate (non-error) JSON files
/// up to this size are written in one go; larger ones stream as usual.
const MAX_JSON_SNIFF_BYTES: u64 = 1_000_000;

/// A redirect that ended on the Moodle login page means the URL was not
/// accessible with the web-service token (e.g. a stale plain
/// `/pluginfile.php` link that escaped the rewrite) — report it instead of
/// saving the login page as the "file".
fn ensure_not_login_page(resp: &reqwest::Response) -> Result<(), MoodleError> {
    if resp.url().path().contains("/login/") {
        return Err(MoodleError::InvalidBody(
            "the file request was redirected to the Moodle login page — the URL is not \
             accessible with the web-service token; use the file URL exactly as returned \
             by get_course_overview / get_assignment / get_discussion_posts"
                .into(),
        ));
    }
    Ok(())
}

/// `webservice/pluginfile.php` answers with HTTP 200 and a JSON error body
/// (`{"error":…,"errorcode":…}`) when the token is invalid/expired or the
/// service forbids file downloads — detect it so the error payload is never
/// mistaken for the file's content. Legitimate JSON attachments (no
/// `errorcode` key) pass through.
fn moodle_json_error(content_type: &str, bytes: &[u8]) -> Option<MoodleError> {
    if !content_type.starts_with("application/json") {
        return None;
    }
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let errorcode = v.get("errorcode")?.as_str()?.to_string();
    let error = v
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("Moodle rejected the file request")
        .to_string();
    Some(MoodleError::Api {
        exception: "webservice".into(),
        errorcode,
        message: error,
    })
}

/// The response's media type, normalized to `type/subtype` (parameters
/// after `;` stripped, defaults to `application/octet-stream`). Shared by
/// fetch_file and download_file.
fn content_type_of(resp: &reqwest::Response) -> String {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string()
}

/// Fetch a Moodle file and return its content as **markdown** for the agent
/// to read. PDF/DOCX/PPTX/XLSX/ODT/RTF/EPUB/CSV are converted with anydoc;
/// plain text passes through; anything else (zip, images…) cannot be
/// converted — download it instead. The web-service token is injected into
/// `pluginfile.php` URLs automatically.
pub async fn fetch_file(client: &MoodleClient, url: &str) -> Result<Value, MoodleError> {
    let resp = client.download_file(url).await?;
    let status = resp.status().as_u16();
    let content_type = content_type_of(&resp);

    if !(200..300).contains(&status) {
        return Err(MoodleError::InvalidBody(format!(
            "file fetch returned HTTP {status}; the URL may be expired or the file inaccessible"
        )));
    }
    ensure_not_login_page(&resp)?;
    // Size guard BEFORE buffering: reject anything the declared size says is
    // too big. Unknown/chunked lengths are enforced mid-stream by read_capped
    // below (a missing Content-Length must not become a full in-RAM buffer).
    if let Some(len) = resp.content_length()
        && len > MAX_FETCH_INPUT_BYTES
    {
        return Err(MoodleError::InvalidBody(format!(
            "file is {len} bytes — too large to fetch inline (limit {MAX_FETCH_INPUT_BYTES}); \
             use download_file to save it to disk instead"
        )));
    }
    let bytes = read_capped(resp, MAX_FETCH_INPUT_BYTES).await?;
    let size = bytes.len();
    if let Some(e) = moodle_json_error(&content_type, &bytes) {
        return Err(e);
    }

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
        // Conversion is CPU-bound and synchronous — run it on the blocking
        // pool. Anydoc is in-memory regardless (its path API is just
        // fs::read + to_markdown_bytes), so the bytes move straight in: no
        // temp file (a predictable name is a symlink/TOCTOU hazard on
        // multi-user machines), no clone, no disk round-trip. The extension
        // fallback covers signature-less formats (CSV).
        let detected =
            anydoc::Format::from_bytes(&bytes).or_else(|| anydoc::Format::from_extension(&ext));
        let ext_owned = ext.clone();
        let conv = tokio::task::spawn_blocking(move || -> Result<String, MoodleError> {
            let format = detected.ok_or_else(|| {
                MoodleError::InvalidBody(format!(
                    "unrecognized document content (.{ext_owned}) — use download_file to save the original"
                ))
            })?;
            anydoc::to_markdown_bytes(&bytes, format).map_err(|e| {
                MoodleError::InvalidBody(format!(
                    "document conversion failed ({e}) — use download_file to save the original"
                ))
            })
        })
        .await
        .map_err(|e| MoodleError::Config(format!("conversion task failed: {e}")))??;
        let md = conv;
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

    // HTML resources → markdown (same promise as the document converters
    // above; a raw `<div>…` blob is not something an agent should read).
    let is_html = content_type.starts_with("text/html") || matches!(ext.as_str(), "html" | "htm");
    if is_html
        && size <= MAX_TEXT_BYTES
        && let Some(md) = std::str::from_utf8(&bytes)
            .ok()
            .map(crate::util::html_to_markdown)
    {
        let text = truncate_chars(&md, MAX_FETCH_CHARS);
        return Ok(json!({
            "url": url,
            "content_type": content_type,
            "size_bytes": size,
            "format": "html",
            "chars": text.chars().count(),
            "markdown": text,
        }));
    }

    // Plain text (or decodable small payloads) → inline.
    if size <= MAX_TEXT_BYTES
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

fn save_create_err(save_to: &str, e: std::io::Error) -> MoodleError {
    if e.kind() == std::io::ErrorKind::NotFound
        && let Some(parent) = std::path::Path::new(save_to).parent()
        && !parent.as_os_str().is_empty()
    {
        return MoodleError::Config(format!(
            "cannot create {save_to}: the directory {} does not exist — \
             create the directory first or save to an existing one",
            parent.display()
        ));
    }
    MoodleError::Config(format!("failed to create {save_to}: {e}"))
}

/// `download_file` — write a Moodle file to a local path. Uses the
/// web-service token (injected into `pluginfile.php` URLs when missing).
/// Returns metadata only; binary payloads never enter the conversation.
pub async fn download_file(
    client: &MoodleClient,
    url: &str,
    save_to: &str,
    overwrite: bool,
) -> Result<Value, MoodleError> {
    // Fail fast on a blank destination — no point downloading first.
    if save_to.trim().is_empty() {
        return Err(MoodleError::Config(
            "save_to is empty — provide the local file path to write the download to".into(),
        ));
    }
    // The overwrite is opt-in: an agent re-running a download (or picking
    // the wrong save_to) must not destroy local files silently — the upload
    // side already rejects same-name files (filenameexist), so the
    // destructive direction gets the same guard.
    if !overwrite && tokio::fs::try_exists(save_to).await.unwrap_or(false) {
        return Err(MoodleError::Config(format!(
            "{save_to} already exists — refusing to overwrite it; pick a new path, delete the \
             existing file first, or pass overwrite=true to replace it intentionally"
        )));
    }
    let mut resp = client.download_file(url).await?;
    let status = resp.status().as_u16();
    let content_type = content_type_of(&resp);

    if !(200..300).contains(&status) {
        return Err(MoodleError::InvalidBody(format!(
            "download returned HTTP {status}; the URL may be expired or the file inaccessible"
        )));
    }
    ensure_not_login_page(&resp)?;
    // Small JSON responses are buffered and checked against Moodle's error
    // payload shape before anything hits the disk; a real (non-error) JSON
    // file of ≤1 MB is then written in one go.
    if content_type.starts_with("application/json")
        && resp
            .content_length()
            .is_some_and(|l| l <= MAX_JSON_SNIFF_BYTES)
    {
        let bytes = read_capped(resp, MAX_JSON_SNIFF_BYTES).await?;
        if let Some(e) = moodle_json_error(&content_type, &bytes) {
            return Err(e);
        }
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::File::create(save_to)
            .await
            .map_err(|e| save_create_err(save_to, e))?;
        file.write_all(&bytes)
            .await
            .map_err(|e| MoodleError::Config(format!("failed to write {save_to}: {e}")))?;
        file.flush()
            .await
            .map_err(|e| MoodleError::Config(format!("failed to flush {save_to}: {e}")))?;
        return Ok(json!({
            "url": url,
            "status": status,
            "content_type": content_type,
            "size_bytes": bytes.len(),
            "saved_to": save_to,
        }));
    }
    // Stream to disk in chunks (no full-body buffering) with a hard cap;
    // a mid-stream overage aborts and removes the partial file.
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(save_to)
        .await
        .map_err(|e| save_create_err(save_to, e))?;
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

/// Upload a local file into the user's draft file area (read from disk —
/// binary data never enters the conversation). `filename` defaults to the
/// path's file name. Returns the draft `item_id` referencing the upload.
pub async fn upload_file_from_path(
    client: &MoodleClient,
    path: &str,
    filename: Option<&str>,
    item_id: i64,
) -> Result<Value, MoodleError> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| MoodleError::Config(format!("failed to stat {path}: {e}")))?;
    if meta.len() > MAX_UPLOAD_BYTES {
        return Err(MoodleError::Config(format!(
            "{path} is {} bytes — over the {}-byte upload limit (Moodle sites cap uploads \
             far below this anyway)",
            meta.len(),
            MAX_UPLOAD_BYTES
        )));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| MoodleError::Config(format!("failed to read {path}: {e}")))?;
    let name = filename
        .map(str::to_string)
        .unwrap_or_else(|| basename(path));

    let files: Value = client.upload_to_draft(&name, bytes, item_id).await?;
    let first = files
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| MoodleError::InvalidBody("upload.php returned no file records".into()))?;
    Ok(json!({
        "uploaded": true,
        "item_id": first.get("itemid").and_then(Value::as_i64).unwrap_or(item_id),
        "filename": first.get("filename").cloned().unwrap_or(json!(name)),
        "filesize": first.get("filesize").cloned().unwrap_or(Value::Null),
    }))
}

fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Read the response body up to `cap` bytes. Streams with `chunk()` so that
/// oversized payloads with no (or lying) Content-Length are rejected
/// mid-transfer instead of being buffered whole into memory.
async fn read_capped(mut resp: reqwest::Response, cap: u64) -> Result<Vec<u8>, MoodleError> {
    let mut buf: Vec<u8> = Vec::new();
    let mut remaining = cap;
    while let Some(chunk) = resp.chunk().await.map_err(MoodleError::from)? {
        if chunk.len() as u64 > remaining {
            return Err(MoodleError::InvalidBody(format!(
                "file exceeds the {cap}-byte inline fetch limit — use download_file \
                 to save it to disk instead"
            )));
        }
        remaining -= chunk.len() as u64;
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::config::Config;

    #[tokio::test]
    async fn fetch_file_converts_csv_in_memory() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/report.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string("name,score\nalice,98\n"))
            .mount(&server)
            .await;
        let client = MoodleClient::new(Config {
            base_url: server.uri(),
            token: "t".into(),
            timeout_secs: 10,
            timezone: None,
            read_only: false,
            disabled_tools: Vec::new(),
            user_agent: None,
            allow_external_fetch: false,
            enable_ws_function: false,
        })
        .unwrap();
        let out = fetch_file(&client, &format!("{}/report.csv", server.uri()))
            .await
            .unwrap();
        assert_eq!(out["format"], "csv");
        assert!(out["markdown"].as_str().unwrap().contains("alice"));
    }

    #[tokio::test]
    async fn read_capped_rejects_oversized_stream() {
        // No Content-Length declared — only the mid-stream cap can stop it.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/big.bin"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("x".repeat(5000))
                    .insert_header("transfer-encoding", "chunked"),
            )
            .mount(&server)
            .await;
        let client = MoodleClient::new(Config {
            base_url: server.uri(),
            token: "t".into(),
            timeout_secs: 10,
            timezone: None,
            read_only: false,
            disabled_tools: Vec::new(),
            user_agent: None,
            allow_external_fetch: false,
            enable_ws_function: false,
        })
        .unwrap();
        let resp = client
            .download_file(&format!("{}/big.bin", server.uri()))
            .await
            .unwrap();
        let err = read_capped(resp, 100).await.expect_err("must reject");
        assert!(err.to_string().contains("100"), "{err:?}");
    }
}

#[cfg(test)]
mod overwrite_tests {
    use super::*;
    use crate::client::MoodleClient;
    use crate::config::Config;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn download_file_refuses_overwrite_unless_flagged() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/f.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"payload".to_vec()))
            .mount(&server)
            .await;
        let client = MoodleClient::new(Config {
            base_url: server.uri(),
            token: "t".into(),
            timeout_secs: 10,
            timezone: None,
            read_only: false,
            disabled_tools: Vec::new(),
            user_agent: None,
            allow_external_fetch: false,
            enable_ws_function: false,
        })
        .unwrap();
        let dir = std::env::temp_dir().join(format!("mdl-ow-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let target = dir.join("existing.txt");
        tokio::fs::write(&target, b"old").await.unwrap();
        let url = format!("{}/f.bin", server.uri());

        let err = download_file(&client, &url, target.to_str().unwrap(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"old");

        download_file(&client, &url, target.to_str().unwrap(), true)
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"payload");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
