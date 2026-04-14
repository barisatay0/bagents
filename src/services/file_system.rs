use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::models::file_modification::FileModification;

/// Apply a list of file modifications to the workspace.
///
/// Each file's content is normalized: literal `\n`, `\t`, and `\"` escape
/// sequences produced by LLMs are always expanded into real characters.
/// The guard condition used previously (`!contains('\n')`) was fragile and
/// caused silent failures — normalization is now unconditional.
pub fn apply_modifications(
    config: &Config,
    modifications: Vec<FileModification>,
) -> Result<(), String> {
    for modif in modifications {
        let full_path = config.workspace_dir.join(&modif.file_path);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let content = unescape_llm_output(&modif.new_content);
        fs::write(&full_path, &content).map_err(|e| e.to_string())?;

        info!(path = %modif.file_path, "File written to workspace");
    }
    Ok(())
}

/// Build a formatted string listing every non-hidden, non-build file in the workspace.
pub fn get_repo_tree(config: &Config) -> String {
    let mut paths: Vec<String> = Vec::new();
    collect_paths(&config.workspace_dir, &config.workspace_dir, &mut paths);
    paths.sort();

    let mut out = String::from("REPOSITORY STRUCTURE (DIRECTORY TREE):\n");
    for p in &paths {
        out.push_str(&format!("  - {}\n", p));
    }
    out
}

/// Read a list of relative file paths from the workspace and return their contents.
/// Files larger than 15 000 bytes are skipped with a notice.
pub fn read_specific_files(config: &Config, files: Vec<String>) -> String {
    let mut out = String::from("REQUESTED FILE CONTENTS:\n");

    for file_path in files {
        let full_path = config.workspace_dir.join(&file_path);
        match fs::read_to_string(&full_path) {
            Ok(content) if content.len() > 15_000 => {
                warn!(path = %file_path, "File too large — skipping");
                out.push_str(&format!(
                    "\n--- FILE: {} (content too large, skipped) ---\n",
                    file_path
                ));
            }
            Ok(content) => {
                debug!(path = %file_path, bytes = content.len(), "File read");
                out.push_str(&format!("\n--- FILE: {} ---\n{}\n", file_path, content));
            }
            Err(_) => {
                out.push_str(&format!(
                    "\n--- FILE: {} (could not read or does not exist) ---\n",
                    file_path
                ));
            }
        }
    }

    out
}

// ── internal helpers ─────────────────────────────────────────────────────────

/// Expand JSON-style escape sequences that LLMs commonly emit in string fields.
///
/// This is always applied — the previous conditional check on `contains('\n')`
/// was unreliable because a file with a real newline early in the content (e.g.
/// a blank comment line) would pass the guard and leave literal `\n` sequences
/// in the written file.
fn unescape_llm_output(s: &str) -> String {
    s.replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\\"", "\"")
}

fn collect_paths(dir: &Path, workspace: &Path, paths: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules" | "dist") {
            continue;
        }

        if path.is_dir() {
            collect_paths(&path, workspace, paths);
        } else {
            let rel = path
                .strip_prefix(workspace)
                .unwrap_or(&path)
                .display()
                .to_string();
            paths.push(rel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescape_always_runs() {
        // Even if the string already has a real newline early on,
        // remaining literal \n sequences are still expanded.
        let input = "line1\nstill has \\n literal and \\t tab and \\\" quote";
        let out = unescape_llm_output(input);
        assert!(out.contains('\n'));
        assert!(out.contains('\t'));
        assert!(out.contains('"'));
        assert!(!out.contains("\\n"));
        assert!(!out.contains("\\t"));
        assert!(!out.contains("\\\""));
    }
}
