use std::env;
use std::path::PathBuf;

/// Holds all validated configuration loaded once at startup.
/// Avoids repeated `env::var` calls and panics scattered across services.
#[derive(Debug, Clone)]
pub struct Config {
    pub openai_api_key: String,
    pub github_token: String,
    pub llm_max_retries: u32,
}

impl Config {
    /// Load and validate all required environment variables.
    /// Returns a descriptive error listing every missing key at once.
pub fn from_env() -> Result<Self, String> {
    let openai_api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY environment variable is not set".to_string())?;
    let github_token = std::env::var("GITHUB_TOKEN")
        .map_err(|_| "GITHUB_TOKEN environment variable is not set".to_string())?;
    let llm_max_retries = std::env::var("LLM_MAX_RETRIES")
        .map_err(|_| "LLM_MAX_RETRIES environment variable is not set".to_string())
        .and_then(|s| {
            s.parse::<u32>()
                .map_err(|_| "LLM_MAX_RETRIES must be a valid u32".to_string())
        })
        .unwrap_or(5u32);
    Ok(Self {
        openai_api_key,
        github_token,
        llm_max_retries,
    })
}
}