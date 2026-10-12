use std::fmt;

/// Runtime configuration, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the Moodle site, e.g. `https://moodle.example.com`.
    pub base_url: String,
    /// Moodle web service token (`wstoken`).
    pub token: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let base_url = std::env::var("MOODLE_URL")
            .map_err(|_| "MOODLE_URL is required (e.g. https://moodle.example.com)".to_string())?;
        let base_url = base_url.trim_end_matches('/').to_string();
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
        Ok(Self {
            base_url,
            token,
            timeout_secs,
        })
    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Config({} [token hidden], timeout={}s)",
            self.base_url, self.timeout_secs
        )
    }
}
