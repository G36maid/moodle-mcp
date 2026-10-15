/// Runtime configuration, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the Moodle site, e.g. `https://moodle.example.com`.
    pub base_url: String,
    /// Moodle web service token (`wstoken`).
    pub token: String,
    /// Request timeout in seconds; also caps the startup timezone lookup.
    pub timeout_secs: u64,
    /// IANA name from MOODLE_TIMEZONE; typos fail at load. `None` = ask Moodle.
    pub timezone: Option<String>,
    /// Hide all write/upload/escape-hatch tools (MOODLE_READ_ONLY=1/true/
    /// yes/on) — read-only access for cautious setups.
    pub read_only: bool,
    /// Additional tool names to hide (MOODLE_DISABLE_TOOLS, comma-separated);
    /// unknown names warn at startup instead of failing it.
    pub disabled_tools: Vec<String>,
    /// Optional User-Agent override (MOODLE_UA). Defaults to the honest
    /// `moodle-mcp/{version}`; the override exists for institutional-filtering
    /// edge cases, not as a stealth feature.
    pub user_agent: Option<String>,
    /// Allow the file tools to fetch URLs outside the configured site
    /// (MOODLE_ALLOW_EXTERNAL_FETCH). Default off: the MCP process makes
    /// requests the user's agent harness cannot see or gate, so its network
    /// footprint stays inside the Moodle site.
    pub allow_external_fetch: bool,
}

/// Normalize a site URL and enforce the https policy: schemeless input
/// defaults to https; plain http is rejected unless the host is loopback
/// (local test instances) or `allow_http` is set (intranet-only
/// deployments). Used by both `serve` (MOODLE_URL) and `auth` (--url / prompt)
/// so the two entry points cannot drift apart.
pub fn normalize_site_url(raw: &str, allow_http: bool) -> Result<String, String> {
    let trimmed = raw.trim();
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", trimmed.trim_end_matches('/'))
    };
    let url =
        url::Url::parse(&with_scheme).map_err(|e| format!("invalid site URL '{trimmed}': {e}"))?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() == "http" && !is_loopback_host(host) && !allow_http {
        return Err(format!(
            "refusing plain http for '{trimmed}' — the token (and on `auth`, the password) \
             would travel unencrypted; use https:// (local test instance? use \
             http://localhost, or set MOODLE_ALLOW_HTTP=1 / --allow-http)"
        ));
    }
    Ok(with_scheme)
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host.starts_with("127.") || host == "::1" || host == "[::1]"
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let raw_url = std::env::var("MOODLE_URL")
            .map_err(|_| "MOODLE_URL is required (e.g. https://moodle.example.com)".to_string())?;
        // Plain http is rejected here (not just warned about): the token — and
        // on `auth`, the password — would otherwise travel in cleartext.
        // Loopback hosts are exempt so local Docker test instances work
        // without flags; MOODLE_ALLOW_HTTP covers intranet-only http sites.
        let allow_http = std::env::var("MOODLE_ALLOW_HTTP")
            .ok()
            .is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"));
        let base_url =
            normalize_site_url(&raw_url, allow_http).map_err(|e| format!("MOODLE_URL: {e}"))?;
        let token = std::env::var("MOODLE_TOKEN")
            .map_err(|_| "MOODLE_TOKEN is required (Moodle web service token)".to_string())?;
        let timeout_secs = std::env::var("MOODLE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let timezone = std::env::var("MOODLE_TIMEZONE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if let Some(tz) = &timezone
            && crate::util::parse_timezone(tz).is_err()
        {
            return Err(format!(
                "MOODLE_TIMEZONE: '{}' is not an IANA timezone name (e.g. Asia/Taipei)",
                tz
            ));
        }
        let read_only = std::env::var("MOODLE_READ_ONLY").ok().is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
        let disabled_tools = std::env::var("MOODLE_DISABLE_TOOLS")
            .ok()
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let user_agent = std::env::var("MOODLE_UA")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let allow_external_fetch = std::env::var("MOODLE_ALLOW_EXTERNAL_FETCH")
            .ok()
            .is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"));
        Ok(Self {
            base_url,
            token,
            timeout_secs,
            timezone,
            read_only,
            disabled_tools,
            user_agent,
            allow_external_fetch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemeless_defaults_to_https() {
        assert_eq!(
            normalize_site_url("moodle3.ntnu.edu.tw", false).unwrap(),
            "https://moodle3.ntnu.edu.tw"
        );
        assert_eq!(
            normalize_site_url("moodle3.ntnu.edu.tw/", false).unwrap(),
            "https://moodle3.ntnu.edu.tw"
        );
    }

    #[test]
    fn https_passes_through() {
        assert_eq!(
            normalize_site_url("https://moodle.example.com", false).unwrap(),
            "https://moodle.example.com"
        );
    }

    #[test]
    fn plain_http_rejected_unless_loopback_or_opt_in() {
        assert!(normalize_site_url("http://moodle.example.com", false).is_err());
        assert_eq!(
            normalize_site_url("http://moodle.example.com", true).unwrap(),
            "http://moodle.example.com"
        );
        assert_eq!(
            normalize_site_url("http://localhost:8080", false).unwrap(),
            "http://localhost:8080"
        );
        assert_eq!(
            normalize_site_url("http://127.0.0.1:8000", false).unwrap(),
            "http://127.0.0.1:8000"
        );
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(normalize_site_url("not a url", false).is_err());
    }
}
