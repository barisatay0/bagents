use serde::{Deserialize, Serialize};

/// JSON response produced by the code reviewer agent.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewerResponse {
    #[serde(default)]
    pub thought_process: String,
    pub is_approved: bool,
    pub feedback_thread: Option<String>,
}
