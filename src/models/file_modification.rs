use serde::{Deserialize, Serialize};

/// A single file to be written (created or overwritten) by a developer agent.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileModification {
    /// Relative path inside the workspace (e.g. `src/lib.rs`).
    pub file_path: String,
    #[serde(default)]
    pub new_content: String,
    #[serde(default)]
    pub search_block: String,
    #[serde(default)]
    pub replace_block: String,
}
