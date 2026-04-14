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
        })
    }
}
