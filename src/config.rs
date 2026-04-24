use std::env;
use std::path::PathBuf;

/// Holds all validated configuration loaded once at startup.
/// Avoids repeated `env::var` calls and panics scattered across services.
#[derive(Debug, Clone)]
pub struct Config {
    pub github_token: String,
    pub github_owner: String,
    pub github_repo: String,
    pub workspace_dir: PathBuf,
    pub llm_api_key: String,
    pub llm_api_url: String,
    pub llm_model: String,
    pub llm_temperature: f32,
    pub json_mode: String,
    pub verify_command: String,
    /// Max tokens for normal (planner / reviewer) requests.
    pub llm_max_tokens: u32,
    /// Max tokens for developer agent requests that write full file content.
    pub llm_max_tokens_large: u32,
}

impl Config {
    /// Load and validate all required environment variables.
    /// Returns a descriptive error listing every missing key at once.
    pub fn from_env() -> Result<Self, String> {
        let mut errors: Vec<String> = Vec::new();

        macro_rules! require {
            ($key:expr) => {
                env::var($key).unwrap_or_else(|_| {
                    errors.push(format!("  - {} is missing", $key));
                    String::new()
                })
            };
        }

        macro_rules! optional_u32 {
            ($key:expr, $default:expr) => {
                env::var($key)
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or($default)
            };
        }

        let github_token = require!("GITHUB_TOKEN");
        let github_owner = require!("GITHUB_OWNER");
        let github_repo = require!("GITHUB_REPO");
        let workspace_raw = require!("WORKSPACE_DIR");
        let llm_api_url = require!("LLM_API_URL");
        let llm_model = require!("LLM_MODEL");

        let llm_api_key = env::var("LLM_API_KEY").unwrap_or_default();
        let llm_temperature: f32 = env::var("LLM_TEMPERATURE")
            .unwrap_or_else(|_| "0.2".to_string())
            .parse()
            .unwrap_or(0.2);

        let json_mode = env::var("LLM_JSON_MODE").unwrap_or_else(|_| "openai".to_string());
        let verify_command = env::var("VERIFY_COMMAND").unwrap_or_default();

        // Token limits — generous defaults so agents never silently truncate.
        // Most frontier models support 8 192 – 32 768 output tokens.
        // Override via .env if your provider has a lower ceiling.
        let llm_max_tokens = optional_u32!("LLM_MAX_TOKENS", 4096);
        let llm_max_tokens_large = optional_u32!("LLM_MAX_TOKENS_LARGE", 8192);

        if !errors.is_empty() {
            return Err(format!(
                "Configuration errors — fix your .env file:\n{}",
                errors.join("\n")
            ));
        }

        let workspace_dir = PathBuf::from(&workspace_raw);
        if !workspace_dir.exists() {
            return Err(format!(
                "WORKSPACE_DIR '{}' does not exist on disk.",
                workspace_raw
            ));
        }

        Ok(Self {
            github_token,
            github_owner,
            github_repo,
            workspace_dir,
            llm_api_key,
            llm_api_url,
            llm_model,
            llm_temperature,
            json_mode,
            verify_command,
            llm_max_tokens,
            llm_max_tokens_large,
        })
    }
}
