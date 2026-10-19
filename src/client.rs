use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
    /// Precomputed once: every call() would otherwise re-format the same
    /// constant endpoint.
    rest_url: String,
    token: String,
    allow_external_fetch: bool,
    /// Local cooldown after a 429/503, keyed by wsfunction:
    /// `(status, until epoch secs)`. While active, `call()` fails locally
    /// without touching the network — an agent's blind retry loop becomes
    /// fast local failures instead of a
    /// request storm against the school's server. Keyed per function:
    /// one throttled function must not blind every tool — other
    /// endpoints (and other tools' writes) keep working through the
    /// window.
    cooldown: Arc<Mutex<HashMap<String, (u16, i64)>>>,
}

/// Redirect confinement: only redirects that ORIGINATE from the Moodle site
/// are host-locked — those carry the injected `?token=` and must never cross
/// to a third party. Redirect chains that started on an external URL (course
/// links → CDNs, http→https hops) may continue. Site-originated chains may
/// also never drop to plain http: the token-bearing query would cross the
/// wire in cleartext even on the same host.
fn redirect_policy(base_host: Option<String>) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        let origin = attempt.previous().last();
        let from_site = origin
            .and_then(|u| u.host_str())
            .is_some_and(|h| Some(h) == base_host.as_deref());
        if from_site
            && downgrades_https(
                origin.map(|u| u.scheme().to_string()).as_deref(),
                attempt.url(),
            )
        {
            return attempt.error(
                "refusing an https→http downgrade redirect \
                                  from the Moodle site",
            );
        }
        match (from_site, attempt.url().host_str()) {
            (true, Some(h)) if Some(h) != base_host.as_deref() => attempt.stop(),
            _ => attempt.follow(),
        }
    })
}

/// An https request may never be redirected to plain http — a stripping
/// proxy or misconfigured school mirror would otherwise carry the token
/// query onto the wire in cleartext.
fn downgrades_https(from_scheme: Option<&str>, to: &url::Url) -> bool {
    from_scheme == Some("https") && to.scheme() == "http"
}

impl MoodleClient {
    pub fn new(cfg: Config) -> Result<Self, MoodleError> {
        let base = url::Url::parse(&cfg.base_url)
            .map_err(|e| MoodleError::Config(format!("invalid MOODLE_URL: {e}")))?;
        let base_host = base.host_str().map(str::to_string);
        // MOODLE_UA overrides for institutional-filtering edge cases; the
        // honest default stays moodle-mcp/{version}.
        let ua = cfg
            .user_agent
            .clone()
            .unwrap_or_else(|| concat!("moodle-mcp/", env!("CARGO_PKG_VERSION")).to_string());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs))
            .redirect(redirect_policy(base_host.clone()))
            .user_agent(&ua)
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
            .user_agent(&ua)
            .build()
            .map_err(|e| MoodleError::Config(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            download_http,
            rest_url: format!(
                "{}/webservice/rest/server.php",
                cfg.base_url.trim_end_matches('/')
            ),
            base_url: cfg.base_url,
            token: cfg.token,
            allow_external_fetch: cfg.allow_external_fetch,
            cooldown: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn parse_retry_after(header: Option<&reqwest::header::HeaderValue>) -> Option<u64> {
        header?.to_str().ok()?.trim().parse::<u64>().ok()
    }

    fn start_cooldown(&self, wsfunction: &str, status: u16, retry_after: u64) {
        let until = crate::util::now_unix() + retry_after as i64;
        // A panic while holding the gate must not poison it shut forever —
        // every later call would fail without ever reaching the wire.
        // Expired entries leave with the insert so the map stays the size
        // of the functions actually in cooldown.
        let mut map = self.cooldown.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, &mut (_, u)| u > crate::util::now_unix());
        map.insert(wsfunction.to_string(), (status, until));
    }

    /// Host identity for same-site checks: hostname plus the explicit port
    /// when non-default. Scheme is ignored (old sites serve both), and
    /// default ports normalize away (http:80 == https:443 host-wise).
    fn host_key(url: &url::Url) -> Option<String> {
        let host = url.host_str()?;
        Some(match url.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        })
    }

    /// Host identity of the configured site (host:port) for same-site checks.
    pub fn base_host_key(&self) -> Option<String> {
        Self::host_key(&url::Url::parse(&self.base_url).ok()?)
    }

    pub fn allow_external_fetch(&self) -> bool {
        self.allow_external_fetch
    }

    /// The file tools only fetch from the Moodle site itself: the MCP
    /// process makes requests the user's agent harness cannot see or gate,
    /// so its network footprint stays inside the site — plus the canonical
    /// wwwroot host (from site info) for alias-configured sites.
    /// MOODLE_ALLOW_EXTERNAL_FETCH opts out. Scheme is not compared: older
    /// sites may serve both http and https.
    pub fn ensure_same_site(
        &self,
        file_url: &str,
        siteurl: Option<&str>,
    ) -> Result<(), MoodleError> {
        if self.allow_external_fetch {
            return Ok(());
        }
        let url = url::Url::parse(file_url)
            .map_err(|e| MoodleError::Config(format!("invalid file URL '{file_url}': {e}")))?;
        let target = Self::host_key(&url).unwrap_or_else(|| "unknown host".into());
        let allowed = [
            self.base_host_key(),
            siteurl
                .and_then(|s| url::Url::parse(s).ok())
                .and_then(|u| Self::host_key(&u)),
        ];
        if allowed.contains(&Some(target.clone())) {
            return Ok(());
        }
        Err(MoodleError::Config(format!(
            "only same-site Moodle file URLs are supported ('{target}' is not part of this site) — \
             for external links use your own web tooling, or set MOODLE_ALLOW_EXTERNAL_FETCH=1"
        )))
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
            let code = value
                .get("errorcode")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(MoodleError::Config(token_error_hint(code, err, service)));
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
        let mut form: Vec<(String, String)> = vec![
            ("wstoken".into(), self.token.clone()),
            ("wsfunction".into(), wsfunction.to_string()),
            ("moodlewsrestformat".into(), "json".into()),
        ];
        flatten_json(params, "", &mut form);

        // Local cooldown gate, per function: after a 429/503 on this
        // function we stop hitting the wire entirely — the request is
        // refused without leaving the process.
        let now = crate::util::now_unix();
        if let Some(&(status, until)) = self
            .cooldown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(wsfunction)
            && now < until
        {
            return Err(MoodleError::HttpCooldown {
                status,
                retry_after: (until - now) as u64,
            });
        }

        let resp = self
            .http
            .post(self.rest_url.as_str())
            .form(&form)
            .send()
            .await
            .map_err(MoodleError::from)?;

        let status = resp.status();
        let retry_after = Self::parse_retry_after(resp.headers().get(reqwest::header::RETRY_AFTER));
        let text = resp.text().await.map_err(MoodleError::from)?;
        tracing::debug!(wsfunction, %status, bytes = text.len(), "moodle ws response");
        tracing::trace!(body = truncate(&text, 1000), "moodle ws body");
        let parsed: Option<Value> = serde_json::from_str(&text).ok();

        // 429/503 start the cooldown regardless of body shape — the back-off
        // signal comes from the status line (proxies/WAFs in front of Moodle
        // send HTML error pages; Moodle maintenance sends 503 + Retry-After).
        if matches!(status.as_u16(), 429 | 503) {
            let secs = retry_after.unwrap_or(if status.as_u16() == 503 { 300 } else { 30 });
            self.start_cooldown(wsfunction, status.as_u16(), secs);
        }

        // Moodle exception payloads can arrive with any status (the error
        // convention is HTTP 200, but proxies/gateways may alter it) — keep
        // mapping them first.
        if parsed
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|obj| obj.contains_key("exception"))
        {
            let exc = serde_json::from_value(parsed.unwrap()).unwrap_or(MoodleException {
                exception: "unknown".into(),
                errorcode: "unknown".into(),
                message: truncate(&text, 200).to_string(),
            });
            return Err(MoodleError::from_exception(exc));
        }

        if !status.is_success() {
            let code = status.as_u16();
            if matches!(code, 429 | 503 | 502 | 504) {
                let secs = retry_after.unwrap_or(if code == 503 { 300 } else { 30 });
                return Err(MoodleError::HttpStatus {
                    status: code,
                    retry_after: secs,
                });
            }
            return Err(MoodleError::NonJsonResponse(format!(
                "expected JSON, got HTTP {status}: {}",
                truncate(&text, 300)
            )));
        }

        let value = parsed.ok_or_else(|| {
            MoodleError::NonJsonResponse(format!(
                "expected JSON, got HTTP {status}: {}",
                truncate(&text, 300)
            ))
        })?;
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
    ///
    /// Token handling in the webservice endpoint:
    /// https://github.com/moodle/moodle/blob/v4.5.4/webservice/pluginfile.php#L47
    /// The app's same-site rewrite:
    /// https://github.com/moodlehq/moodleapp/blob/9d52da9b4e1decbaff1e4de89971b658e040be2e/src/core/static/url.ts#L671-L673
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
        // https://github.com/moodle/moodle/blob/v4.5.4/webservice/upload.php#L109
        // https://github.com/moodle/moodle/blob/v4.5.4/webservice/upload.php#L159
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

/// login/token.php reports a machine `errorcode` next to the human `error`
/// text; each code has a different fix, so name it instead of offering the
/// same generic advice for everything. Codes not known here keep the site's
/// own error text.
fn token_error_hint(code: &str, error: &str, service: &str) -> String {
    match code {
        "invalidlogin" => "login failed: invalid username or password. If your school signs \
         in through SSO (university email, and so on), your account may \
         have no local Moodle password — ask your Moodle admin for a token."
            .to_string(),
        "enablewsdescription" => "login failed: web services are disabled on this Moodle site — \
         only the site admin can turn them on (Site administration > \
         General > Advanced features > Enable web services)."
            .to_string(),
        "servicenotavailable" => format!(
            "login failed: the web service '{service}' is not available to \
         your account — check the --service spelling; if you meant the \
         default, it is moodle_mobile_app (if mobile access is disabled \
         site-wide, only the admin can re-enable it)."
        ),
        "cannotcreatetoken" => "login failed: your account is not allowed to create web service \
         tokens — hardened school sites often remove this permission. Ask \
         your Moodle admin to create a token for you (Site administration \
         > Server > Web services > Manage tokens) or to grant the \
         'Create web service tokens' capability to the Authenticated user \
         role."
            .to_string(),
        "missingrequiredcapability" => format!(
            "login failed: the service '{service}' requires a capability your \
         account lacks — ask your Moodle admin what to use instead."
        ),
        "requirecorrectaccess" => {
            "login failed: this site restricts mobile access by IP address — \
         ask your Moodle admin to allowlist your network."
                .to_string()
        }
        _ => format!(
            "login failed: {} (check username/password and that the \
         service '{service}' is enabled for your account)",
            crate::util::echo_safe(error)
        ),
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

/// If the site itself redirects elsewhere (schools migrating
/// example.edu -> moodle.example.edu lose the path on the way), the final
/// origin is what actually serves web services — calling the old one means
/// our POST lands on the homepage as a GET. Returns the final origin when
/// it differs; network errors return None so callers keep the original URL
/// rather than failing here.
pub async fn resolve_site_redirect(base_url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let resp = client.get(base_url).send().await.ok()?;
    let final_origin = format_origin(resp.url());
    // Adopt a migration only when the scheme survives: a hijacked or
    // misconfigured redirect to plain http would otherwise move every later
    // credential POST onto the wire in cleartext. Same-scheme keeps
    // loopback/http test instances working.
    let original_scheme = url::Url::parse(base_url).ok()?.scheme().to_string();
    if !adoptable_scheme(&original_scheme, resp.url().scheme()) {
        return None;
    }
    if origin_changed(base_url, resp.url().as_str()) {
        Some(final_origin)
    } else {
        None
    }
}

/// A redirect may move the site to another https host (moodle.example.edu
/// -> moodle3.example.edu); it may never drag an https origin down to
/// plain http.
fn adoptable_scheme(original: &str, final_scheme: &str) -> bool {
    final_scheme == "https" || final_scheme == original
}

fn format_origin(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

fn origin_changed(old: &str, new: &str) -> bool {
    let (Some(old), Some(new)) = (
        url::Url::parse(old).ok().as_ref().map(format_origin),
        url::Url::parse(new).ok().as_ref().map(format_origin),
    ) else {
        return false;
    };
    old != new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_error_hints_name_the_actual_fix() {
        for (code, want) in [
            ("invalidlogin", "no local Moodle password"),
            ("enablewsdescription", "Advanced features"),
            ("servicenotavailable", "moodle_mobile_app"),
            ("cannotcreatetoken", "Manage tokens"),
            ("missingrequiredcapability", "capability"),
            ("requirecorrectaccess", "allowlist"),
        ] {
            let hint = token_error_hint(code, "site error text", "moodle_mobile_app");
            assert!(hint.contains(want), "{code}: want '{want}' in {hint}");
            assert!(hint.starts_with("login failed:"), "{code}: {hint}");
        }
        // unknown codes keep the site's own error text
        assert!(
            token_error_hint("somethingnew", "site error text", "svc").contains("site error text")
        );
    }

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
                user_agent: None,
                allow_external_fetch: false,
                enable_ws_function: false,
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

#[cfg(test)]
mod backoff_tests {
    use super::*;
    use crate::config::Config;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(uri: &str) -> MoodleClient {
        MoodleClient::new(Config {
            base_url: uri.into(),
            token: "t".into(),
            timeout_secs: 10,
            timezone: None,
            read_only: false,
            disabled_tools: Vec::new(),
            user_agent: None,
            allow_external_fetch: false,
            enable_ws_function: false,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn json_429_without_exception_is_not_success() {
        // A front component (Cloudflare-style) 429 with a JSON body used to be
        // returned as a successful response: no `exception` key, status ignored.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webservice/rest/server.php"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(serde_json::json!({"success": false, "errors": []})),
            )
            .mount(&server)
            .await;
        let client = client_for(&server.uri());
        let err = client
            .call("core_webservice_get_site_info", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, MoodleError::HttpStatus { status: 429, .. }),
            "{err}"
        );
        // The cooldown gate now rejects the next call locally.
        let err = client
            .call("core_webservice_get_site_info", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, MoodleError::HttpCooldown { status: 429, .. }),
            "{err}"
        );
    }

    #[tokio::test]
    async fn retry_after_header_is_honored() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webservice/rest/server.php"))
            .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "120"))
            .mount(&server)
            .await;
        let client = client_for(&server.uri());
        let err = client
            .call("core_webservice_get_site_info", &serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("retry after 120s"), "{err}");
    }

    #[tokio::test]
    async fn moodle_exception_on_non_200_still_maps_without_cooldown() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webservice/rest/server.php"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "exception": "moodle_exception",
                "errorcode": "dberror",
                "message": "database is sad"
            })))
            .expect(2)
            .mount(&server)
            .await;
        let client = client_for(&server.uri());
        for _ in 0..2 {
            let err = client
                .call("core_webservice_get_site_info", &serde_json::json!({}))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("dberror"), "{err}");
        }
        // 500 is not a back-off signal: both calls reached the wire.
    }
}

#[cfg(test)]
mod same_site_tests {
    use super::*;
    use crate::config::Config;

    fn client_for(base: &str, allow_external: bool) -> MoodleClient {
        MoodleClient::new(Config {
            base_url: base.into(),
            token: "t".into(),
            timeout_secs: 10,
            timezone: None,
            read_only: false,
            disabled_tools: Vec::new(),
            user_agent: None,
            allow_external_fetch: allow_external,
            enable_ws_function: false,
        })
        .unwrap()
    }

    #[test]
    fn file_urls_must_be_same_site_unless_opted_out() {
        let strict = client_for("https://moodle.example.com", false);
        // Same host: OK.
        assert!(
            strict
                .ensure_same_site("https://moodle.example.com/pluginfile.php/1/f.pdf", None)
                .is_ok()
        );
        // Canonical wwwroot host (alias-configured sites): OK.
        assert!(
            strict
                .ensure_same_site(
                    "https://wwwroot.example.org/pluginfile.php/1/f.pdf",
                    Some("https://wwwroot.example.org")
                )
                .is_ok()
        );
        // Foreign host: rejected even when a siteurl is known.
        assert!(
            strict
                .ensure_same_site(
                    "https://evil.example.com/x",
                    Some("https://wwwroot.example.org")
                )
                .is_err()
        );
        // Scheme-lenient: http target against an https base is still same-site.
        assert!(
            strict
                .ensure_same_site("http://moodle.example.com/pluginfile.php/1/f.pdf", None)
                .is_ok()
        );
        // Opt-out allows anything.
        assert!(
            client_for("https://moodle.example.com", true)
                .ensure_same_site("https://evil.example.com/x", None)
                .is_ok()
        );
    }
}
