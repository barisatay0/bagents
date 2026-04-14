use serde::{Deserialize, Serialize};

/// JSON response produced by the team lead agent.
#[derive(Debug, Serialize, Deserialize)]
pub struct TeamLeaderResponse {
    pub thought_process: String,
    /// One of: `"backend_dev"`, `"frontend_dev"`, `"devops_dev"`.
    pub assigned_agent: String,
    pub architectural_plan: String,
    /// Relative file paths the developer agent should read before writing code.
    pub files_to_read: Vec<String>,

    /// List of specific function/class names to extract using Tree-sitter.
    #[serde(default)]
    pub chunks_to_read: Vec<String>,
}
