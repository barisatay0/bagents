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

        // ── Full-file mode ────────────────────────────────────────────────
        if !modif.new_content.is_empty() {
            let content = unescape_llm_output(&modif.new_content);
            fs::write(&full_path, &content).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "File written (full-rewrite mode)");
            continue;
        }

        // ── Patch mode ────────────────────────────────────────────────────
        if modif.search_block.is_empty() {
            return Err(format!(
                "FileModification for '{}' has neither new_content nor search_block.",
                modif.file_path
            ));
        }

        let existing = fs::read_to_string(&full_path)
            .map_err(|e| format!("Could not read '{}' for patching: {}", modif.file_path, e))?;

        let search = unescape_llm_output(&modif.search_block);
        let replace = unescape_llm_output(&modif.replace_block);

        // 1. Exact match
        if existing.contains(&search) {
            let patched = existing.replacen(&search, &replace, 1);
            fs::write(&full_path, patched).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "Patch applied (exact match)");
            continue;
        }

        // 2. CRLF-normalised fallback — file might have \r\n from Windows checkout
        let existing_norm = existing.replace("\r\n", "\n");
        let search_norm = search.replace("\r\n", "\n");
        if existing_norm.contains(&search_norm) {
            let patched = existing_norm.replacen(&search_norm, &replace, 1);
            fs::write(&full_path, patched).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "Patch applied (CRLF-normalised)");
            continue;
        }

        // 3. Trim-each-line fallback — handles leading/trailing whitespace drift
        let trim_lines =
            |s: &str| -> String { s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n") };
        let existing_trim = trim_lines(&existing_norm);
        let search_trim = trim_lines(&search_norm);
        if existing_trim.contains(&search_trim) {
            let patched = existing_trim.replacen(&search_trim, &replace, 1);
            fs::write(&full_path, patched).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "Patch applied (trim-normalised)");
            continue;
        }

        // 4. THE MAGIC BULLET: Whitespace-agnostic fuzzy match
        if let Some((start_idx, end_idx)) = find_fuzzy_match_range(&existing, &search) {
            let patched = format!(
                "{}{}{}",
                &existing[..start_idx],
                replace,
                &existing[end_idx..]
            );
            fs::write(&full_path, patched).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "Patch applied (fuzzy match - whitespace ignored)");
            continue;
        }

        return Err(format!(
            "search_block not found in '{}' after all matching attempts.",
            modif.file_path
        ));
    }
    Ok(())
}

fn find_fuzzy_match_range(existing: &str, search: &str) -> Option<(usize, usize)> {
    let search_chars: Vec<char> = search.chars().filter(|c| !c.is_whitespace()).collect();
    if search_chars.is_empty() {
        return None;
    }

    let existing_indices: Vec<(usize, char)> = existing
        .char_indices()
        .filter(|(_, c)| !c.is_whitespace())
        .collect();

    if search_chars.len() > existing_indices.len() {
        return None;
    }

    for i in 0..=(existing_indices.len() - search_chars.len()) {
        let mut matches = true;
        for j in 0..search_chars.len() {
            if existing_indices[i + j].1 != search_chars[j] {
                matches = false;
                break;
            }
        }

        if matches {
            let start_byte = existing_indices[i].0;
            let last_matched_char = existing_indices[i + search_chars.len() - 1];
            let end_byte = last_matched_char.0 + last_matched_char.1.len_utf8();
            return Some((start_byte, end_byte));
        }
    }

    None
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
            Ok(content) if content.len() > 50_000 => {
                warn!(path = %file_path, "File too large — skipping");
                out.push_str(&format!(
                    "\n--- FILE: {} (too large, skipped) ---\n",
                    file_path
                ));
            }
            Ok(content) => {
                debug!(path = %file_path, bytes = content.len(), "File read");
                out.push_str(&format!("\n--- FILE: {} ---\n{}\n", file_path, content));
            }
            Err(_) => {
                out.push_str(&format!("\n--- FILE: {} (not found) ---\n", file_path));
            }
        }
    }

    out
}

pub fn read_semantic_outlines(config: &Config, files: Vec<String>) -> String {
    let mut out = String::from("SEMANTIC FILE OUTLINES:\n");
    for file_path in files {
        let full_path = config.workspace_dir.join(&file_path);
        match fs::read_to_string(&full_path) {
            Ok(content) => {
                let chunks =
                    crate::services::semantic::extract_semantic_chunks(&file_path, &content);

                if chunks.is_empty() {
                    out.push_str(&format!(
                        "\n--- FILE: {} (no structure found) ---\n",
                        file_path
                    ));
                } else {
                    out.push_str(&format!("\n--- FILE: {} ---\n", file_path));
                    for chunk in chunks {
                        out.push_str(&format!(
                            "  - [{}] {} (Lines {} - {})\n",
                            chunk.kind, chunk.name, chunk.start_line, chunk.end_line
                        ));
                    }
                }
            }
            Err(_) => out.push_str(&format!("\n--- FILE: {} (not found) ---\n", file_path)),
        }
    }

    out
}

pub fn read_specific_chunks(config: &Config, file_path: &str, chunk_names: Vec<String>) -> String {
    let full_path = config.workspace_dir.join(file_path);
    let mut out = format!("SPECIFIC CODE BLOCKS FROM {}:\n", file_path);

    match fs::read_to_string(&full_path) {
        Ok(content) => {
            let chunks = crate::services::semantic::extract_semantic_chunks(file_path, &content);
            for target_name in chunk_names {
                if let Some(chunk) = chunks.iter().find(|c| c.name == target_name) {
                    out.push_str(&format!(
                        "\n--- {} [{}] ---\n{}\n",
                        chunk.name, chunk.kind, chunk.content
                    ));
                } else {
                    out.push_str(&format!(
                        "\n--- Error: Chunk '{}' not found ---\n",
                        target_name
                    ));
                }
            }
        }
        Err(e) => out.push_str(&format!("Error reading file: {}\n", e)),
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

        // Skip hidden files, build artifacts, and massive lock files
        if name.starts_with('.')
            || name.ends_with(".lock")
            || name == "node_modules"
            || name == "target"
            || name == "dist"
            || name == "vendor"
            || name == "out"
        {
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
