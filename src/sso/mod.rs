//! SSO token acquisition via the Moodle mobile-app launch flow.
//!
//! Schools that federate login (OIDC/SAML portals) give accounts no local
//! Moodle password, so [`crate::client::MoodleClient::fetch_token`] cannot
//! authenticate them — the password belongs to the identity provider, and
//! `login/token.php` answers `invalidlogin`. The Moodle mobile app already
//! solves this: `admin/tool/mobile/launch.php` takes a browser session that
//! has completed the school's SSO, generates a web-service token for that
//! user, and hands it over as a `moodlemobile://token=BASE64` redirect
//! (base64 of `site_id:::token[:::private_token]`, where
//! `site_id = md5(wwwroot . passport)`).
//!
//! A terminal tool rides the same flow without an app or a browser: each
//! school's driver under `src/sso/` replays its own login chain over plain
//! HTTP, and [`sso_login`] dispatches to the driver registered for the
//! site's host. One self-contained file per school — extract a shared
//! family module only when a second school proves the same shape.

use base64::Engine;

/// Build the launch URL. With `confirmed=false` Moodle answers a bare 302
/// whose Location header carries the token — what an HTTP client wants.
/// With `confirmed=true` it renders the iOS-style page whose `#launchapp`
/// anchor holds the same scheme URL, so a human can copy it from a browser.
pub fn launch_page_url(base_url: &str, service: &str, passport: &str, confirmed: bool) -> String {
    format!(
        "{}/admin/tool/mobile/launch.php?service={service}&passport={passport}\
         &urlscheme=moodlemobile{}",
        base_url.trim_end_matches('/'),
        if confirmed { "&confirmed=1" } else { "" }
    )
}

/// Extract the web-service token from a `moodlemobile://token=…` launch URL
/// (or from the bare base64 payload). Tolerates URL-encoded padding, query
/// suffixes and a trailing private-token segment.
pub fn extract_launch_token(input: &str) -> Result<String, String> {
    let input = input.trim();
    let payload = match input.find("token=") {
        Some(i) => {
            let rest = &input[i + "token=".len()..];
            let end = rest
                .find(|c: char| c == '&' || c.is_whitespace())
                .unwrap_or(rest.len());
            &rest[..end]
        }
        None => input,
    };
    if payload.is_empty() {
        return Err("no token payload found — paste the moodlemobile:// URL \
                    shown on the launch page"
            .into());
    }

    // Schools sit behind proxies that re-encode the scheme URL; base64's
    // padding (+, /, =) is the part that gets percent-escaped.
    let payload = payload
        .replace("%3D", "=")
        .replace("%3d", "=")
        .replace("%2B", "+")
        .replace("%2b", "+")
        .replace("%2F", "/")
        .replace("%2f", "/");

    let mut padded = payload.to_string();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .map_err(|e| {
            format!(
                "could not decode the launch payload ({e}) — copy the whole \
                 moodlemobile://token=… link from the launch page"
            )
        })?;
    let decoded = String::from_utf8(decoded)
        .map_err(|_| "the decoded launch payload is not UTF-8".to_string())?;

    let mut parts = decoded.splitn(3, ":::");
    let _site_id = parts.next().unwrap_or_default();
    match parts.next() {
        Some(token) if !token.is_empty() => Ok(token.to_string()),
        _ => Err("the launch payload does not contain a token (expected \
             site_id:::token) — make sure you copied the moodlemobile://token=… \
             link, not the page URL"
            .into()),
    }
}

/// Check a token actually works by asking the site for its own identity —
/// the same call `whoami` uses, so a failure here means the token is wrong,
/// expired or restricted, not that the API surface is broken. The token
/// travels in the POST body, never the query string — queries land in
/// access logs.
pub async fn validate_token(base_url: &str, token: &str) -> Result<serde_json::Value, String> {
    let url = format!(
        "{}/webservice/rest/server.php",
        base_url.trim_end_matches('/')
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("could not prepare request: {e}"))?;
    let value: serde_json::Value = client
        .post(&url)
        .form(&[
            ("wstoken", token),
            ("wsfunction", "core_webservice_get_site_info"),
            ("moodlewsrestformat", "json"),
        ])
        .send()
        .await
        .map_err(|e| format!("could not reach {url} ({e})"))?
        .json()
        .await
        .map_err(|e| format!("the site returned non-JSON ({e})"))?;
    if value.get("exception").is_some() {
        return Err(format!(
            "the site rejected the token: {}",
            echo_safe(
                value
                    .get("message")
                    .or_else(|| value.get("error"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown error")
            )
        ));
    }
    Ok(value)
}

/// Server-provided text reaches the terminal through error messages —
/// strip control characters (escape-sequence injection) and cap length.
fn echo_safe(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// The loopback carve-out for the plaintext-credential guard: local test
/// instances (the same exemption `MOODLE_ALLOW_HTTP` makes) plus
/// localhost-shaped names.
pub(crate) fn is_loopback_host(host: Option<&str>) -> bool {
    let Some(host) = host else { return false };
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// The result of a successful SSO run: the token for `MOODLE_TOKEN` and the
/// site identity that was verified with it.
pub struct SsoOutcome {
    pub token: String,
    pub site_info: serde_json::Value,
}

mod ntust;

/// Run the SSO driver registered for this site's host.
///
/// Each school lives in its own self-contained module under `src/sso/`
/// (metadata, host match, and the whole login chain); adding a school is
/// one file plus one arm here. No driver for this host? The caller falls
/// back to the credentials mode or sends the user to their admin —
/// guessing a flow for an unknown school would leak passwords to the
/// wrong form.
pub async fn sso_login(
    base_url: &str,
    service: &str,
    username: &str,
    password: &str,
) -> Result<SsoOutcome, String> {
    let host = url::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    if ntust::matches(&host) {
        return ntust::login(base_url, service, username, password).await;
    }
    Err(format!(
        "no SSO driver for {host} — use the credentials mode (login/token.php) \
         or ask your Moodle admin for a token"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as B64;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn payload(site: &str, token: &str) -> String {
        B64.encode(format!("{site}:::{token}"))
    }

    #[test]
    fn extracts_from_scheme_url_and_variants() {
        let p = payload("sitesha", "tok123");
        // plain scheme form
        assert_eq!(
            extract_launch_token(&format!("moodlemobile://token={p}")).unwrap(),
            "tok123"
        );
        // URL-encoded padding survives proxies
        assert_eq!(
            extract_launch_token(&format!(
                "moodlemobile://token={}%3D%3D",
                p.trim_end_matches('=')
            ))
            .unwrap(),
            "tok123"
        );
        // query suffix after the payload
        assert_eq!(
            extract_launch_token(&format!("moodlemobile://token={p}&from=qr")).unwrap(),
            "tok123"
        );
        // bare payload pasted without the scheme
        assert_eq!(extract_launch_token(&p).unwrap(), "tok123");
        // private token segment is ignored
        let with_private = B64.encode("sitesha:::tok123:::private9");
        assert_eq!(
            extract_launch_token(&format!("moodlemobile://token={with_private}")).unwrap(),
            "tok123"
        );
    }

    #[test]
    fn rejects_payloads_without_a_token() {
        let only_site = B64.encode("justsiteid");
        assert!(extract_launch_token(&format!("moodlemobile://token={only_site}")).is_err());
        assert!(extract_launch_token("not base64 at all!").is_err());
        assert!(extract_launch_token("").is_err());
    }

    #[test]
    fn launch_url_points_at_the_confirmation_page() {
        let url = launch_page_url(
            "https://moodle.example.com/",
            "moodle_mobile_app",
            "pp",
            true,
        );
        assert!(url.starts_with("https://moodle.example.com/admin/tool/mobile/launch.php?"));
        assert!(url.contains("service=moodle_mobile_app"));
        assert!(url.contains("confirmed=1"));
        // the machine flow wants the bare 302 instead
        assert!(
            !launch_page_url(
                "https://moodle.example.com",
                "moodle_mobile_app",
                "pp",
                false
            )
            .contains("confirmed")
        );
    }

    #[tokio::test]
    async fn validate_token_reports_site_identity_and_rejections() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/webservice/rest/server.php"))
            .and(body_string_contains("wstoken=good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sitename": "School", "fullname": "Ming"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/webservice/rest/server.php"))
            .and(body_string_contains("wstoken=bad"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "exception": "moodle_exception", "message": "Invalid token"
            })))
            .mount(&server)
            .await;

        let ok = validate_token(&server.uri(), "good").await.unwrap();
        assert_eq!(ok["fullname"], "Ming");
        let err = validate_token(&server.uri(), "bad").await.unwrap_err();
        assert!(err.contains("Invalid token"), "{err}");
    }
}
