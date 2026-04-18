use std::env;
use std::path::PathBuf;

/// Holds all validated configuration loaded once at startup.
/// Avoids repeated `env::var` calls and panics scattered across services.
#[derive(Debug, Clone)]
pub struct Config {
    pub openai_api_key: String,
    pub openai_model: String,
    pub llm_max_retries: u32,
}

impl Config {
    /// Load and validate all required environment variables.
    /// Returns a descriptive error listing every missing key at once.
pub fn from_env() -> Result<Self, String> {
    let openai_api_key = std::env::var("OPENAI_API_KEY")
        .map_err(|_| "OPENAI_API_KEY environment variable is not set".to_string())?;
    
    let openai_model = std::env::var("OPENAI_MODEL")
        .unwrap_or_else(|_| "gpt-4-turbo".to_string());
    
    let llm_max_retries = std::env::var("LLM_MAX_RETRIES")
        .map(|s| s.parse::<u32>().unwrap_or(5u32))
        .unwrap_or(5u32);
    
    Ok(Self {
        openai_api_key,
        openai_model,
        llm_max_retries,
    })
}
}