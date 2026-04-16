use std::path::PathBuf;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub github_owner: String,
    pub github_repo: String,
    pub github_token: String,
    pub workspace_dir: PathBuf,
    pub llm_model: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        // ... existing implementation ...
    }

    pub fn get_masked_token(&self) -> String {
        let chars: Vec<char> = self.github_token.chars().collect();
        if chars.len() <= 8 {
            "***REDACTED***".to_string()
        } else {
            format!("{}...{}", 
                &self.github_token[0..4],
                &self.github_token[self.github_token.len() - 4..]
            )
        }
    }
}