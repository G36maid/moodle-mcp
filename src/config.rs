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
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let base_url = std::env::var("MOODLE_URL")
            .map_err(|_| "MOODLE_URL is required (e.g. https://moodle.example.com)".to_string())?
            .trim_end_matches('/')
            .to_string();
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err(format!(
                "MOODLE_URL must start with http(s)://, got: {base_url}"
            ));
        }
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
        Ok(Self {
            base_url,
            token,
            timeout_secs,
            timezone,
            read_only,
            disabled_tools,
        })
    }
}
