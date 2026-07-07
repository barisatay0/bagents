use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub tracker_type: String,
    pub tracker_url: String,
    pub tracker_token: String,
    pub tracker_username: Option<String>,
    pub tracker_project: String,
    pub repo_type: String,
    pub repo_url: String,
    pub repo_token: String,
    pub repo_project: String,
    pub workspace_dir: PathBuf,
    pub llm_api_key: String,
    pub llm_api_url: String,
    pub llm_model: String,
    pub llm_temperature: f32,
    pub json_mode: String,
    pub verify_command: String,
    pub llm_max_tokens: u32,
    pub llm_max_tokens_large: u32,
    pub base_branch: String,
    pub poll_interval_secs: u64,
    pub error_retry_secs: u64,
}

impl Config {
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

        let tracker_type = env::var("TRACKER_TYPE").unwrap_or_else(|_| "github".to_string());
        let tracker_url = env::var("TRACKER_URL").unwrap_or_else(|_| {
            if tracker_type == "gitlab" { "https://gitlab.com".to_string() } else { "https://api.github.com".to_string() }
        });
        let tracker_token = require!("TRACKER_TOKEN");
        let tracker_username = env::var("TRACKER_USERNAME").ok();
        let tracker_project = require!("TRACKER_PROJECT");

        let repo_type = env::var("REPO_TYPE").unwrap_or_else(|_| "github".to_string());
        let repo_url = env::var("REPO_URL").unwrap_or_else(|_| {
            if repo_type == "gitlab" { "https://gitlab.com".to_string() } else { "https://api.github.com".to_string() }
        });
        let repo_token = require!("REPO_TOKEN");
        let repo_project = require!("REPO_PROJECT");
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

        let llm_max_tokens = optional_u32!("LLM_MAX_TOKENS", 4096);
        let llm_max_tokens_large = optional_u32!("LLM_MAX_TOKENS_LARGE", 8192);

        let base_branch = env::var("BASE_BRANCH").unwrap_or_else(|_| "main".to_string());
        let poll_interval_secs = env::var("POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(30);
        let error_retry_secs = env::var("ERROR_RETRY_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60);

        let valid_tracker_types = vec!["github", "gitlab", "jira"];
        let valid_repo_types = vec!["github", "gitlab", "forgejo"];

        if !valid_tracker_types.contains(&tracker_type.as_str()) {
            errors.push(format!(
                "  - TRACKER_TYPE '{}' is not supported. Supported tracker types are: {:?}",
                tracker_type, valid_tracker_types
            ));
        }

        if !valid_repo_types.contains(&repo_type.as_str()) {
            errors.push(format!(
                "  - REPO_TYPE '{}' is not supported. Supported repo types are: {:?}",
                repo_type, valid_repo_types
            ));
        }

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
            tracker_type,
            tracker_url,
            tracker_token,
            tracker_username,
            tracker_project,
            repo_type,
            repo_url,
            repo_token,
            repo_project,
            workspace_dir,
            llm_api_key,
            llm_api_url,
            llm_model,
            llm_temperature,
            json_mode,
            verify_command,
            llm_max_tokens,
            llm_max_tokens_large,
            base_branch,
            poll_interval_secs,
            error_retry_secs,
        })
    }
}
