use std::time::Duration;

use serde::de::DeserializeOwned;
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
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl MoodleClient {
    pub fn new(cfg: Config) -> Result<Self, MoodleError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .user_agent(concat!("moodle-mcp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| MoodleError::Config(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
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

    /// Call a web service function.
    ///
    /// `params` is a JSON object whose values may contain nested
    /// objects/arrays; they are flattened into Moodle's PHP bracket notation,
    /// e.g. `{"courseids": [2, 5]}` → `courseids[0]=2&courseids[1]=5`.
    pub async fn call<T: DeserializeOwned>(
        &self,
        wsfunction: &str,
        params: &Value,
    ) -> Result<T, MoodleError> {
        let value = self.call_raw(wsfunction, params).await?;
        serde_json::from_value(value).map_err(MoodleError::from)
    }

    /// Call a web service function and return the raw JSON response.
    pub async fn call_raw(&self, wsfunction: &str, params: &Value) -> Result<Value, MoodleError> {
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
        tracing::debug!(
            wsfunction,
            %status,
            bytes = text.len(),
            "moodle ws response"
        );
        tracing::trace!(body = truncate(&text, 1000), "moodle ws body");
        let value: Value = serde_json::from_str(&text).map_err(|_| {
            MoodleError::NonJsonResponse(format!(
                "expected JSON, got HTTP {status}: {}",
                truncate(&text, 300)
            ))
        })?;

        // Moodle reports API errors with HTTP 200 and an exception payload.
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
        Ok(value)
    }

    /// Download bytes from a Moodle file URL, injecting the web service token
    /// ONLY when the URL points at the same Moodle site's `pluginfile.php`
    /// and carries no token. URLs on other hosts are fetched as-is — the
    /// token is never sent to third parties (SSRF/leak protection).
    pub async fn download_file(&self, file_url: &str) -> Result<reqwest::Response, MoodleError> {
        let mut url = url::Url::parse(file_url)
            .map_err(|e| MoodleError::Config(format!("invalid file URL: {e}")))?;
        let base = url::Url::parse(&self.base_url)
            .map_err(|e| MoodleError::Config(format!("invalid base URL: {e}")))?;
        let same_site = url.scheme() == base.scheme()
            && url.host_str() == base.host_str()
            && url.port_or_known_default() == base.port_or_known_default();
        if same_site
            && !url.query_pairs().any(|(k, _)| k == "token")
            && url.path().contains("pluginfile.php")
        {
            url.query_pairs_mut().append_pair("token", &self.token);
        }
        self.http.get(url).send().await.map_err(MoodleError::from)
    }

    /// Build the final download URL: same-site `pluginfile.php` URLs get the
    /// token injected; everything else is passed through untouched.
    pub(crate) fn prepare_download_url(&self, file_url: &str) -> Result<url::Url, MoodleError> {
        let mut url =
            url::Url::parse(file_url).map_err(|e| MoodleError::Config(format!("invalid file URL: {e}")))?;
        let base = url::Url::parse(&self.base_url)
            .map_err(|e| MoodleError::Config(format!("invalid base URL: {e}")))?;
        let same_site = url.scheme() == base.scheme()
            && url.host_str() == base.host_str()
            && url.port_or_known_default() == base.port_or_known_default();
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
        Ok(value)
    }
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
    use serde_json::json;

    #[test]
    fn download_url_token_rules() {
        let client = MoodleClient::new(Config {
            base_url: "https://moodle.example.com".into(),
            token: "secret".into(),
            timeout_secs: 10,
        })
        .unwrap();

        // 同站 pluginfile:注入 token
        let u = client
            .prepare_download_url("https://moodle.example.com/webservice/pluginfile.php/1/f.pdf")
            .unwrap();
        assert!(u.query_pairs().any(|(k, v)| k == "token" && v == "secret"));

        // 已帶 token:不重複
        let u = client
            .prepare_download_url("https://moodle.example.com/webservice/pluginfile.php/1/f.pdf?token=other")
            .unwrap();
        assert_eq!(u.query_pairs().filter(|(k, _)| k == "token").count(), 1);

        // 外部主機:即使路徑含 pluginfile.php 也不注入(token 外洩防護)
        let u = client
            .prepare_download_url("https://evil.example.com/pluginfile.php/1/f.pdf")
            .unwrap();
        assert!(!u.query_pairs().any(|(k, _)| k == "token"));

        // 非本站路徑:不注入
        let u = client
            .prepare_download_url("https://cdn.example.com/files/f.pdf")
            .unwrap();
        assert!(!u.query_pairs().any(|(k, _)| k == "token"));
    }

    #[test]
    fn flattens_php_notation() {
        let mut out = Vec::new();
        flatten_json(
            &json!({"courseids": [2, 5], "criteria": [{"key": "search", "value": "math"}]}),
            "",
            &mut out,
        );
        assert!(out.contains(&("courseids[0]".to_string(), "2".to_string())));
        assert!(out.contains(&("courseids[1]".to_string(), "5".to_string())));
        assert!(out.contains(&("criteria[0][key]".to_string(), "search".to_string())));
        assert!(out.contains(&("criteria[0][value]".to_string(), "math".to_string())));
    }
}
