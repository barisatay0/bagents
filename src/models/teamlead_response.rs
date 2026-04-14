use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct TeamLeaderResponse {
    pub thought_process: String,
    pub assigned_agent: String,
    pub architectural_plan: String,
    pub files_to_read: Vec<String>,
}
