//! Per-school SSO driver for 國立臺灣科技大學 (NTUST).
//!
//! moodle2.ntust.edu.tw — Moodle 4.5 (Build 20260714). Login is federated
//! to the ssoam2.ntust.edu.tw OIDC portal (IdentityServer): accounts have
//! no local Moodle password, so `login/token.php` answers `invalidlogin`
//! whatever the password.
//!
//! The whole flow is replayed here over plain HTTP — no browser, no paste:
//! /auth/oidc/ -> login form (CSRF + ReturnUrl) -> credentials ->
//! form_post code -> Moodle callback -> launch.php 302 -> token.
//! Verified live 2026-10-19 with a student account.
//!
//! If another school turns out to share this exact shape, extract the
//! shared steps into a family module then; one member is not a family.

use super::{SsoOutcome, extract_launch_token, launch_page_url, validate_token};

/// Hosts this driver handles.
pub fn matches(host: &str) -> bool {
    host == "moodle2.ntust.edu.tw"
}

/// Log in through NTUST's OIDC portal (ssoam2) by replaying the portal's
/// own GET/POST chain over plain HTTP — the session (and the token) end up
/// belonging to this process, so no browser and no paste. Every failure
/// names the step that broke.
pub async fn login(
    base_url: &str,
    service: &str,
    username: &str,
    password: &str,
) -> Result<SsoOutcome, String> {
    // Follow redirects like a browser would, except when the school hands
    // the token over via its custom scheme — that 302 is the payload, not
    // something to chase.
    let client = reqwest::Client::builder()
        .user_agent(concat!("moodle-mcp/", env!("CARGO_PKG_VERSION")))
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 10 {
                attempt.error("too many redirects")
            } else if matches!(attempt.url().scheme(), "http" | "https") {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("could not prepare request: {e}"))?;
    let base = base_url.trim_end_matches('/');

    // the Moodle session this chain starts must survive until the launch call
    let login = client
        .get(format!("{base}/auth/oidc/"))
        .send()
        .await
        .map_err(|e| format!("could not start the SSO flow ({e})"))?;
    let login_url = login.url().clone();
    let login_html = login.text().await.map_err(|e| e.to_string())?;
    let (action, mut fields) = forms(&login_html)
        .into_iter()
        .find(|(_, f)| f.iter().any(|(n, _)| n == "__RequestVerificationToken"))
        .ok_or(
            "the school's login page has no recognizable form — this school \
             may need its own SSO driver",
        )?;
    // the page ships empty Username/Password inputs alongside the hidden
    // ones — drop them or the duplicates would shadow the real credentials
    fields.retain(|(n, _)| n != "Username" && n != "Password");
    fields.push(("Username".into(), username.into()));
    fields.push(("Password".into(), password.into()));

    // a successful login answers with the response_mode=form_post page
    // carrying the code
    let post_url = url::Url::parse(login_url.as_str())
        .map_err(|e| format!("bad login URL: {e}"))?
        .join(&action)
        .map_err(|e| format!("bad form action: {e}"))?;
    // The password crosses exactly this connection — never plain http
    // unless the school's host is loopback (the same carve-out
    // MOODLE_ALLOW_HTTP makes for local test instances).
    if post_url.scheme() != "https" && !super::is_loopback_host(post_url.host_str()) {
        return Err(format!(
            "refusing to send the password over {} — the school's login form \
             resolved to a non-https address",
            post_url.scheme()
        ));
    }
    let after = client
        .post(post_url)
        .form(&fields)
        .send()
        .await
        .map_err(|e| format!("the login request failed ({e})"))?;
    let after_url = after.url().clone();
    let after_html = after.text().await.map_err(|e| e.to_string())?;
    let (callback, code_fields) = forms(&after_html)
        .into_iter()
        .find(|(_, f)| f.iter().any(|(n, _)| n == "code"))
        .ok_or(
            "the school rejected the login (wrong password, captcha or an \
             unknown flow) — no authorization code came back",
        )?;

    let callback_url = url::Url::parse(after_url.as_str())
        .map_err(|e| format!("bad callback URL: {e}"))?
        .join(&callback)
        .map_err(|e| format!("bad callback action: {e}"))?;
    client
        .post(callback_url)
        .form(&code_fields)
        .send()
        .await
        .map_err(|e| format!("the Moodle callback failed ({e})"))?;

    // launch.php now answers with the token redirect the policy above stops at
    let passport = format!(
        "mcp{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    let launch = client
        .get(launch_page_url(base, service, &passport, false))
        .send()
        .await
        .map_err(|e| format!("the launch page could not be fetched ({e})"))?;
    let location = launch
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(
            "the school did not issue a token — the session may not be \
             logged in, or the mobile web service is disabled",
        )?
        .to_string();
    let token = extract_launch_token(&location)?;
    let site_info = validate_token(base, &token).await?;
    Ok(SsoOutcome { token, site_info })
}
/// Collect (action, hidden-input fields) for every <form> in an HTML page.
fn forms(html: &str) -> Vec<(String, Vec<(String, String)>)> {
    let form_re = regex::Regex::new("(?is)<form([^>]*)>(.*?)</form>").unwrap();
    let action_re = regex::Regex::new("action=\"([^\"]*)\"").unwrap();
    let input_re = regex::Regex::new("<input[^>]*>").unwrap();
    let name_re = regex::Regex::new("name=\"([^\"]+)\"").unwrap();
    let value_re = regex::Regex::new("value=\"([^\"]*)\"").unwrap();
    let mut out = Vec::new();
    for cap in form_re.captures_iter(html) {
        let action = action_re
            .captures(&cap[1])
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        let mut fields = Vec::new();
        for input in input_re.captures_iter(&cap[2]) {
            let tag = &input[0];
            if let Some(n) = name_re.captures(tag) {
                let v = value_re
                    .captures(tag)
                    .map(|c| c[1].replace("&amp;", "&"))
                    .unwrap_or_default();
                fields.push((n[1].to_string(), v));
            }
        }
        out.push((action, fields));
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The whole driver replayed offline: moodle starts the OIDC flow, the
    /// IdP shows an IdentityServer-style form, the login returns a
    /// form_post page, the callback establishes the session and launch.php
    /// hands the token over via its custom scheme.
    #[tokio::test]
    async fn oidc_sso_token_replays_the_whole_flow() {
        let server = MockServer::start().await;
        let base = server.uri();
        let launch_payload = B64.encode("site1:::goodtoken");
        Mock::given(method("GET"))
            .and(path("/auth/oidc/"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/idp/login"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/idp/login"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "<form method=\"POST\" action=\"/idp/submit\">\
                 <input name=\"__RequestVerificationToken\" value=\"TKN\">\
                 <input name=\"ReturnUrl\" value=\"/idp/authorize?x=1\"></form>",
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/idp/submit"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", "/idp/authorize?x=1"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/idp/authorize"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "<form method=\"POST\" action=\"{base}/auth/oidc/\">\
                 <input name=\"code\" value=\"C1\"><input name=\"state\" value=\"S1\"></form>"
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/auth/oidc/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("signed in"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/admin/tool/mobile/launch.php"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("moodlemobile://token={launch_payload}")),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/webservice/rest/server.php"))
            .and(body_string_contains("wstoken=goodtoken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "sitename": "School", "fullname": "Ming"
            })))
            .mount(&server)
            .await;

        let outcome = login(&base, "moodle_mobile_app", "u", "p")
            .await
            .expect("the driver should complete the offline flow");
        assert_eq!(outcome.token, "goodtoken");
        assert_eq!(outcome.site_info["fullname"], "Ming");
    }
}
