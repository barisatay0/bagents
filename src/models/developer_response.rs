use serde::{Deserialize, Serialize};

use crate::models::file_modification::FileModification;

#[derive(Debug, Serialize, Deserialize)]
pub struct DeveloperResponse {
    pub thought_process: String,
    pub branch_name: String,
    pub files_to_modify: Vec<FileModification>,
}
