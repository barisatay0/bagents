use serde::{Deserialize, Serialize};

/// JSON response produced by the code reviewer agent.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewerResponse {
    pub thought_process: String,
    pub is_approved: bool,
    /// Actionable feedback when `is_approved` is false.
    pub feedback_thread: Option<String>,
}
