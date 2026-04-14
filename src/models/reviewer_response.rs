use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ReviewerResponse {
    pub thought_process: String,
    pub is_approved: bool,
    pub feedback_thread: Option<String>,
}
