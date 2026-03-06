use serde::{Deserialize, Serialize};

// Response format for the Reviewer agent
#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewerResponse {
    pub thought_process: String,
    pub is_approved: bool,
    pub feedback_thread: Option<String>, // Optional: may be empty if approved
}
