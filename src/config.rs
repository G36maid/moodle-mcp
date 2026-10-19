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
    /// Hide all write/upload/escape-hatch tools (MOODLE_READ_ONLY=true) —
    /// read-only access for cautious setups.
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
    /// Expose the raw escape hatch pair `list_ws_functions` +
    /// `call_ws_function` (MOODLE_ENABLE_WS_FUNCTION). Default off: the raw
    /// web-service surface — including writes — must be a conscious opt-in,
    /// and a discovery tool whose calls cannot be executed is a dead end.
    /// MOODLE_READ_ONLY hides `call_ws_function` regardless.
    pub enable_ws_function: bool,
}

/// Parse a boolean env switch: accepts true/false spellings (1/0, yes/no,
/// on/off) case-insensitively; anything else is a startup error. Silent
/// fallback would turn a typo'd safety switch (MOODLE_READ_ONLY=ture) into
/// its default — the one mistake the operator cannot see.
fn parse_bool_flag(name: &str, raw: &str) -> Result<bool, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "{name}: '{}' is not a boolean value — use true or false (1/0, yes/no, on/off also work)",
            raw.trim()
        )),
    }
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
    if url.scheme() == "http" && !crate::util::is_loopback_host(host) && !allow_http {
        return Err(format!(
            "refusing plain http for '{trimmed}' — the token (and on `auth`, the password) \
             would travel unencrypted; use https:// (local test instance? use \
             http://localhost, or set MOODLE_ALLOW_HTTP=1 / --allow-http)"
        ));
    }
    Ok(with_scheme)
}

/// Parse MOODLE_TIMEOUT_SECS: a typo must fail startup, not silently
/// become the default — the same rule the boolean switches follow.
fn parse_timeout_secs(raw: &str) -> Result<u64, String> {
    let secs: u64 = raw.trim().parse().map_err(|_| {
        format!(
            "MOODLE_TIMEOUT_SECS: '{}' is not a number of seconds (e.g. 60)",
            raw.trim()
        )
    })?;
    if secs == 0 {
        return Err("MOODLE_TIMEOUT_SECS: must be at least 1 second".to_string());
    }
    Ok(secs)
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let raw_url = std::env::var("MOODLE_URL")
            .map_err(|_| "MOODLE_URL is required (e.g. https://moodle.example.com)".to_string())?;
        // Plain http is rejected here (not just warned about): the token — and
        // on `auth`, the password — would otherwise travel in cleartext.
        // Loopback hosts are exempt so local Docker test instances work
        // without flags; MOODLE_ALLOW_HTTP covers intranet-only http sites.
        let allow_http = match std::env::var("MOODLE_ALLOW_HTTP") {
            Ok(v) => parse_bool_flag("MOODLE_ALLOW_HTTP", &v)?,
            Err(_) => false,
        };
        let base_url =
            normalize_site_url(&raw_url, allow_http).map_err(|e| format!("MOODLE_URL: {e}"))?;
        let token = std::env::var("MOODLE_TOKEN")
            .map_err(|_| "MOODLE_TOKEN is required (Moodle web service token)".to_string())?;
        let timeout_secs = match std::env::var("MOODLE_TIMEOUT_SECS") {
            Ok(v) => parse_timeout_secs(&v)?,
            Err(_) => 60,
        };
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
        let read_only = match std::env::var("MOODLE_READ_ONLY") {
            Ok(v) => parse_bool_flag("MOODLE_READ_ONLY", &v)?,
            Err(_) => false,
        };
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
        let allow_external_fetch = match std::env::var("MOODLE_ALLOW_EXTERNAL_FETCH") {
            Ok(v) => parse_bool_flag("MOODLE_ALLOW_EXTERNAL_FETCH", &v)?,
            Err(_) => false,
        };
        let enable_ws_function = match std::env::var("MOODLE_ENABLE_WS_FUNCTION") {
            Ok(v) => parse_bool_flag("MOODLE_ENABLE_WS_FUNCTION", &v)?,
            Err(_) => false,
        };
        Ok(Self {
            base_url,
            token,
            timeout_secs,
            timezone,
            read_only,
            disabled_tools,
            user_agent,
            allow_external_fetch,
            enable_ws_function,
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
    fn bool_flags_accept_true_and_false_spellings() {
        for yes in ["1", "true", "YES", "On", "  true  "] {
            assert!(parse_bool_flag("X", yes).unwrap(), "{yes} must parse true");
        }
        for no in ["0", "false", "NO", "Off", " 0 "] {
            assert!(!parse_bool_flag("X", no).unwrap(), "{no} must parse false");
        }
        for bad in ["", "ture", "2", "enabled"] {
            assert!(parse_bool_flag("X", bad).is_err(), "{bad} must be rejected");
        }
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

    #[test]
    fn timeout_typo_fails_instead_of_defaulting() {
        // safety-relevant env values must fail loudly — same rule as the
        // boolean switches
        for bad in ["6O", "", "abc", "0", "-5"] {
            assert!(parse_timeout_secs(bad).is_err(), "{bad} must be rejected");
        }
        assert_eq!(parse_timeout_secs("90").unwrap(), 90);
        assert_eq!(parse_timeout_secs(" 60 ").unwrap(), 60);
    }

    #[test]
    fn loopback_detection_matches_host_shapes() {
        for host in [
            "localhost",
            "moodle.localhost",
            "127.0.0.1",
            "127.8.9.10",
            "::1",
        ] {
            assert!(crate::util::is_loopback_host(host), "{host} is loopback");
        }
        for host in ["example.com", "localhost.evil.com", "10.0.0.1"] {
            assert!(!crate::util::is_loopback_host(host), "{host} is not");
        }
    }
}
