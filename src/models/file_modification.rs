use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct FileModification {
    pub file_path: String,
    pub new_content: String,
}
