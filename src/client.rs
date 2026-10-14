use std::time::Duration;

use serde_json::Value;

use crate::config::Config;
use crate::error::{MoodleError, MoodleException};

/// Client for the Moodle Web Services REST protocol.
///
/// All calls go through the single RPC-style endpoint:
/// `POST {base}/webservice/rest/server.php` with form-urlencoded parameters
/// (`wstoken`, `wsfunction`, `moodlewsrestformat=json`, plus the function's
/// own parameters in PHP array notation).
#[derive(Clone)]
pub struct MoodleClient {
    /// RPC client — total request timeout applies (API calls are small).
    http: reqwest::Client,
    /// File-transfer client — NO total timeout (large downloads stream for
    /// minutes); a read timeout catches stalled connections instead.
    download_http: reqwest::Client,
    base_url: String,
    token: String,
}

/// Redirect confinement: only redirects that ORIGINATE from the Moodle site
/// are host-locked — those carry the injected `?token=` and must never cross
/// to a third party. Redirect chains that started on an external URL (course
/// links → CDNs, http→https hops) may continue.
fn redirect_policy(base_host: Option<String>) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        let from_site = attempt
            .previous()
            .last()
            .and_then(|u| u.host_str())
            .is_some_and(|h| Some(h) == base_host.as_deref());
        match (from_site, attempt.url().host_str()) {
            (true, Some(h)) if Some(h) != base_host.as_deref() => attempt.stop(),
            _ => attempt.follow(),
        }
    })
}

impl MoodleClient {
    pub fn new(cfg: Config) -> Result<Self, MoodleError> {
        let base = url::Url::parse(&cfg.base_url)
            .map_err(|e| MoodleError::Config(format!("invalid MOODLE_URL: {e}")))?;
        let base_host = base.host_str().map(str::to_string);
        let ua = concat!("moodle-mcp/", env!("CARGO_PKG_VERSION"));
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .redirect(redirect_policy(base_host.clone()))
            .user_agent(ua)
            .build()
            .map_err(|e| MoodleError::Config(format!("failed to build HTTP client: {e}")))?;
        // File transfers: the total timeout above would abort any large
        // download mid-stream. Instead bound only connection setup and the
        // gap between reads, so stalled links still die but slow big files
        // finish.
        let download_http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(cfg.timeout_secs.max(60)))
            .redirect(redirect_policy(base_host))
            .user_agent(ua)
            .build()
            .map_err(|e| MoodleError::Config(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            download_http,
            base_url: cfg.base_url,
            token: cfg.token,
        })
    }

    /// Generate a web service token via `POST /login/token.php`.
    ///
    /// This is the standard credential flow (`service` usually
    /// `moodle_mobile_app`). On success returns the raw JSON payload
    /// (`token` / `privatetoken`); on failure Moodle replies with
    /// `{error, errorcode}` which is mapped to a clear message.
    pub async fn fetch_token(
        base_url: &str,
        username: &str,
        password: &str,
        service: &str,
    ) -> Result<Value, MoodleError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("moodle-mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| MoodleError::Config(format!("failed to build HTTP client: {e}")))?;
        let url = format!("{}/login/token.php", base_url.trim_end_matches('/'));
        let resp = http
            .post(&url)
            .form(&[
                ("username", username),
                ("password", password),
                ("service", service),
            ])
            .send()
            .await
            .map_err(MoodleError::from)?;
        let text = resp.text().await.map_err(MoodleError::from)?;
        let value: Value = serde_json::from_str(&text).map_err(|_| {
            MoodleError::InvalidBody(format!(
                "login/token.php returned non-JSON: {}",
                truncate(&text, 200)
            ))
        })?;
        if let Some(err) = value.get("error").and_then(Value::as_str) {
            return Err(MoodleError::Config(format!(
                "login failed: {err} (check username/password and that the \
                 service '{service}' is enabled for your account)"
            )));
        }
        Ok(value)
    }

    /// Call a web service function and return the parsed JSON response.
    ///
    /// `params` is a JSON object whose values may contain nested
    /// objects/arrays; they are flattened into Moodle's PHP bracket notation,
    /// e.g. `{"courseids": [2, 5]}` → `courseids[0]=2&courseids[1]=5`.
    /// Moodle reports API errors with HTTP 200 and an `{exception, …}`
    /// payload, which is mapped to [`MoodleError::Api`].
    pub async fn call(&self, wsfunction: &str, params: &Value) -> Result<Value, MoodleError> {
        let url = format!(
            "{}/webservice/rest/server.php",
            self.base_url.trim_end_matches('/')
        );

        let mut form: Vec<(String, String)> = vec![
            ("wstoken".into(), self.token.clone()),
            ("wsfunction".into(), wsfunction.to_string()),
            ("moodlewsrestformat".into(), "json".into()),
        ];
        flatten_json(params, "", &mut form);

        let resp = self
            .http
            .post(&url)
            .form(&form)
            .send()
            .await
            .map_err(MoodleError::from)?;

        let status = resp.status();
        let text = resp.text().await.map_err(MoodleError::from)?;
        tracing::debug!(wsfunction, %status, bytes = text.len(), "moodle ws response");
        tracing::trace!(body = truncate(&text, 1000), "moodle ws body");
        let value: Value = serde_json::from_str(&text).map_err(|_| {
            MoodleError::NonJsonResponse(format!(
                "expected JSON, got HTTP {status}: {}",
                truncate(&text, 300)
            ))
        })?;

        if value
            .as_object()
            .is_some_and(|obj| obj.contains_key("exception"))
        {
            let exc = serde_json::from_value(value).unwrap_or(MoodleException {
                exception: "unknown".into(),
                errorcode: "unknown".into(),
                message: truncate(&text, 200).to_string(),
            });
            return Err(MoodleError::from_exception(exc));
        }
        Ok(value)
    }

    /// Download bytes from a Moodle file URL, injecting the web service token
    /// ONLY when the URL points at the same Moodle site's `pluginfile.php`
    /// and carries no token. URLs on other hosts are fetched as-is — the
    /// token is never sent to third parties (SSRF/leak protection).
    pub async fn download_file(&self, file_url: &str) -> Result<reqwest::Response, MoodleError> {
        let url = self.prepare_download_url(file_url)?;
        self.download_http
            .get(url)
            .send()
            .await
            .map_err(MoodleError::from)
    }

    /// Build the final download URL. Same-site `pluginfile.php` URLs are
    /// rewritten to the token-aware `webservice/pluginfile.php` endpoint
    /// (the official Moodle app does the same — plain `pluginfile.php`
    /// only authenticates browser sessions and 303-redirects to the login
    /// page, which would otherwise be downloaded as the "file"), then the
    /// token is injected if missing. Everything else passes through
    /// untouched.
    pub(crate) fn prepare_download_url(&self, file_url: &str) -> Result<url::Url, MoodleError> {
        let mut url = url::Url::parse(file_url)
            .map_err(|e| MoodleError::Config(format!("invalid file URL: {e}")))?;
        let base = url::Url::parse(&self.base_url)
            .map_err(|e| MoodleError::Config(format!("invalid base URL: {e}")))?;
        let same_site = url.scheme() == base.scheme()
            && url.host_str() == base.host_str()
            && url.port_or_known_default() == base.port_or_known_default();
        if same_site && let Some(rewritten) = rewrite_pluginfile_segment(url.path()) {
            url.set_path(&rewritten);
        }
        if same_site
            && !url.query_pairs().any(|(k, _)| k == "token")
            && url.path().contains("pluginfile.php")
        {
            url.query_pairs_mut().append_pair("token", &self.token);
        }
        Ok(url)
    }

    /// Upload a file to the user's draft file area via `webservice/upload.php`.
    ///
    /// Returns the raw JSON array response (contains `itemid` to reference
    /// the uploaded file in subsequent function calls).
    pub async fn upload_to_draft(
        &self,
        filename: &str,
        bytes: Vec<u8>,
        itemid: i64,
    ) -> Result<Value, MoodleError> {
        let url = format!(
            "{}/webservice/upload.php",
            self.base_url.trim_end_matches('/')
        );
        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename.to_string());
        let form = reqwest::multipart::Form::new()
            .text("token", self.token.clone())
            .text("filearea", "draft")
            .text("itemid", itemid.to_string())
            .part("file_1", part); // Moodle accepts any part name for the first file

        let resp = self
            .http
            .post(&url)
            .multipart(form)
            .send()
            .await
            .map_err(MoodleError::from)?;
        let text = resp.text().await.map_err(MoodleError::from)?;
        let value: Value = serde_json::from_str(&text).map_err(|_| {
            MoodleError::InvalidBody(format!(
                "upload.php returned non-JSON: {}",
                truncate(&text, 300)
            ))
        })?;
        if let Some(obj) = value.as_object()
            && obj.contains_key("exception")
        {
            let exc: MoodleException = serde_json::from_value(value).unwrap_or(MoodleException {
                exception: "unknown".into(),
                errorcode: "unknown".into(),
                message: truncate(&text, 200).to_string(),
            });
            return Err(MoodleError::from_exception(exc));
        }
        // upload.php ALSO reports per-file failures with HTTP 200 and an
        // error record that has no `exception` key (`filenameexist`,
        // `fileoversized`, …). A record carrying a non-empty `error` means
        // the file was NOT stored — surfacing it keeps upload_file from
        // reporting success while the draft area still holds the old file.
        if let Some(records) = value.as_array()
            && let Some(bad) = records.iter().find(|r| {
                r.get("error")
                    .and_then(Value::as_str)
                    .is_some_and(|e| !e.is_empty())
            })
        {
            return Err(MoodleError::Api {
                exception: "upload_failed".into(),
                errorcode: bad
                    .get("errortype")
                    .and_then(Value::as_str)
                    .unwrap_or("uploaderror")
                    .to_string(),
                message: bad
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("upload rejected")
                    .to_string(),
            });
        }
        Ok(value)
    }
}

/// Rewrite `/pluginfile.php/<args…>` into `/webservice/pluginfile.php/<args…>`
/// in a URL path. Works for sites installed at the web root and in
/// subdirectories; leaves paths that already go through
/// `/webservice/pluginfile.php` (or `draftfile.php`, a browser-only area)
/// untouched. Returns `None` when there is nothing to rewrite.
fn rewrite_pluginfile_segment(path: &str) -> Option<String> {
    let segs: Vec<&str> = path.split('/').collect();
    for i in 0..segs.len().saturating_sub(1) {
        if segs[i] == "pluginfile.php"
            && !segs[i + 1].is_empty()
            && segs.get(i.wrapping_sub(1)).copied().unwrap_or("") != "webservice"
        {
            let mut rewritten = segs.clone();
            rewritten[i] = "webservice/pluginfile.php";
            return Some(rewritten.join("/"));
        }
    }
    None
}

/// Flatten a JSON object into `key=value` pairs using PHP bracket notation,
/// e.g. `{"courseids":[2,5],"filter":{"k":"v"}}` →
/// `courseids[0]=2`, `courseids[1]=5`, `filter[k]=v`.
pub fn flatten_json(value: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}[{k}]")
                };
                flatten_json(v, &p, out);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                let p = format!("{prefix}[{i}]");
                flatten_json(v, &p, out);
            }
        }
        Value::Null => {}
        Value::Bool(b) => out.push((prefix.to_string(), if *b { "1".into() } else { "0".into() })),
        Value::Number(n) => out.push((prefix.to_string(), n.to_string())),
        Value::String(s) => out.push((prefix.to_string(), s.clone())),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_download_url_rules() {
        let mk = |base: &str| {
            MoodleClient::new(Config {
                base_url: base.into(),
                token: "secret".into(),
                timeout_secs: 10,
                timezone: None,
                read_only: false,
                disabled_tools: Vec::new(),
            })
            .unwrap()
        };
        let client = mk("https://moodle.example.com");
        let sub = mk("https://example.com/campus");

        // (client, input URL, expected final path, token expected in query)
        let cases: &[(&MoodleClient, &str, &str, bool)] = &[
            // Plain root-level pluginfile URL → rewritten + token injected.
            (
                &client,
                "https://moodle.example.com/pluginfile.php/26/mod_assign/intro/0/HW1.pdf",
                "/webservice/pluginfile.php/26/mod_assign/intro/0/HW1.pdf",
                true,
            ),
            // Site installed in a subdirectory still rewrites.
            (
                &sub,
                "https://example.com/campus/pluginfile.php/1/f.pdf",
                "/campus/webservice/pluginfile.php/1/f.pdf",
                true,
            ),
            // Already the webservice endpoint: no double rewrite, token ensured.
            (
                &client,
                "https://moodle.example.com/webservice/pluginfile.php/1/f.pdf",
                "/webservice/pluginfile.php/1/f.pdf",
                true,
            ),
            // A bare script with no file arguments is left alone (still a
            // same-site pluginfile URL, so the token applies).
            (
                &client,
                "https://moodle.example.com/pluginfile.php",
                "/pluginfile.php",
                true,
            ),
            // draftfile.php is a browser-only area — never rewritten, no token.
            (
                &client,
                "https://moodle.example.com/draftfile.php/5/user/draft/12345/f.txt",
                "/draftfile.php/5/user/draft/12345/f.txt",
                false,
            ),
            // External host: never rewritten, never token-injected
            // (token-leak protection), even with a pluginfile path.
            (
                &client,
                "https://evil.example.com/pluginfile.php/1/f.pdf",
                "/pluginfile.php/1/f.pdf",
                false,
            ),
            // Off-site path without pluginfile: no token.
            (
                &client,
                "https://cdn.example.com/files/f.pdf",
                "/files/f.pdf",
                false,
            ),
        ];
        for (i, (cl, input, want_path, want_token)) in cases.iter().enumerate() {
            let u = cl
                .prepare_download_url(input)
                .unwrap_or_else(|e| panic!("case {i}: {e}"));
            assert_eq!(u.path(), *want_path, "case {i} path");
            assert_eq!(
                u.query_pairs().any(|(k, v)| k == "token" && v == "secret"),
                *want_token,
                "case {i} token"
            );
        }

        // Token already present: exactly one, never duplicated.
        let u = client
            .prepare_download_url(
                "https://moodle.example.com/webservice/pluginfile.php/1/f.pdf?token=other",
            )
            .unwrap();
        assert_eq!(u.query_pairs().filter(|(k, _)| k == "token").count(), 1);
    }
}
