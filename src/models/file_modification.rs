use serde::{Deserialize, Serialize};

/// A single-file edit produced by a developer agent.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileModification {
    /// Relative path inside the workspace (e.g. `src/lib.rs`).
    pub file_path: String,

    /// Full file content. Use ONLY for new files or files under 30 lines.
    #[serde(default)]
    pub new_content: String,

    /// Legacy single search/replace pair. Kept for backwards compatibility.
    #[serde(default)]
    pub search_block: String,

    #[serde(default)]
    pub replace_block: String,

    /// Semantic chunk replacement (preferred for whole-function rewrites).
    #[serde(default)]
    pub target_chunk: String,

    /// Ordered list of surgical SEARCH/REPLACE pairs.
    /// Preferred over `search_block` when making multiple edits to one file
    /// or when only a few lines need changing.
    #[serde(default)]
    pub search_replace_blocks: Vec<SearchReplaceBlock>,
}

/// One SEARCH → REPLACE pair within a file.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchReplaceBlock {
    /// Exact lines to locate in the file — must include 2-3 lines of
    /// unchanged context above and below the edit site.
    pub search: String,
    /// Lines that replace `search`. Write only the changed lines plus the
    /// same context lines included in `search`.
    pub replace: String,
}
