use serde::{Deserialize, Serialize};

// Structure that holds the files to be modified by developer agents
#[derive(Debug, Serialize, Deserialize)]
pub struct FileModification {
    pub file_path: String,
    pub new_content: String,
}
