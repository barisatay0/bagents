use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::models::file_modification::FileModification;

// ── Repository map ────────────────────────────────────────────────────────────

/// Build a high-level repository map using Tree-sitter.
///
/// Unlike `get_repo_tree()` which lists file paths only, this function parses
/// each source file and emits every symbol's signature (name + first line of
/// the declaration) without including any body content.
///
/// The map is compact by design — a 2 000-line file is represented by a handful
/// of one-line entries — so it can be included in every Team Lead prompt without
/// consuming significant token budget.  The Team Lead can then request the full
/// content of specific files via `files_to_read`.
///
/// Files that are too large (> 100 KB), binary, or in unsupported languages
/// fall back to their path-only entry so the map remains complete.
pub fn get_repo_map(config: &Config) -> String {
    let mut file_paths: Vec<String> = Vec::new();
    collect_paths(&config.workspace_dir, &config.workspace_dir, &mut file_paths);
    file_paths.sort();

    let mut out = String::from("REPOSITORY MAP (file paths + symbol signatures):\n\n");

    for rel_path in &file_paths {
        let full_path = config.workspace_dir.join(rel_path);

        // Attempt to read the file; skip silently if unreadable or too large.
        let source = match fs::read_to_string(&full_path) {
            Ok(s) if s.len() <= 100_000 => s,
            Ok(_) => {
                out.push_str(&format!("{}  [too large — skipped]\n", rel_path));
                continue;
            }
            Err(_) => {
                // Binary or unreadable file — list the path only.
                out.push_str(&format!("{}\n", rel_path));
                continue;
            }
        };

        let signatures =
            crate::services::semantic::extract_signatures(rel_path, &source);

        if signatures.is_empty() {
            // No parseable symbols (unsupported language, empty file, plain text,
            // etc.) — emit the path alone so the team lead knows it exists.
            out.push_str(&format!("{}\n", rel_path));
        } else {
            out.push_str(
                &crate::services::semantic::format_file_signatures(rel_path, &signatures),
            );
            out.push('\n');
        }
    }

    out
}

// ── File tree (path-only listing, used in developer input) ────────────────────

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

// ── File content reading ──────────────────────────────────────────────────────

/// Apply a list of file modifications to the workspace.
///
/// Each file's content is normalised: literal `\n`, `\t`, and `\"` escape
/// sequences produced by LLMs are always expanded into real characters.
/// The guard condition used previously (`!contains('\n')`) was fragile and
/// caused silent failures — normalisation is now unconditional.
pub fn apply_modifications(
    config: &Config,
    modifications: Vec<FileModification>,
) -> Result<(), String> {
    for modif in modifications {
        let full_path = config.workspace_dir.join(&modif.file_path);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let existing = fs::read_to_string(&full_path).unwrap_or_default();

        if !modif.target_chunk.is_empty() && !modif.new_content.is_empty() {
            let chunks =
                crate::services::semantic::extract_semantic_chunks(&modif.file_path, &existing);

            if let Some(chunk) = chunks.iter().find(|c| c.name == modif.target_chunk) {
                let lines: Vec<&str> = existing.lines().collect();
                let start_idx = chunk.start_line.saturating_sub(1);
                let end_idx = chunk.end_line;

                let mut patched = String::new();

                if start_idx > 0 {
                    patched.push_str(&lines[..start_idx].join("\n"));
                    patched.push('\n');
                }

                patched.push_str(&unescape_llm_output(&modif.new_content));

                if end_idx < lines.len() {
                    patched.push('\n');
                    patched.push_str(&lines[end_idx..].join("\n"));
                }

                fs::write(&full_path, patched).map_err(|e| e.to_string())?;
                info!(path = %modif.file_path, chunk = %modif.target_chunk, "Patch applied (Semantic Chunking)");
                continue;
            } else {
                let available_chunks: Vec<&str> = chunks.iter().map(|c| c.name.as_str()).collect();
                return Err(format!(
                    "CRITICAL: Semantic chunk '{}' not found in '{}'. Available chunks are: {:?}. \
                     If you are trying to ADD a completely new function, do NOT use target_chunk. \
                     Instead, use search_replace_blocks or search_block to find the location and insert your new code there.",
                    modif.target_chunk, modif.file_path, available_chunks
                ));
            }
        }

        // ── 2. Full-file mode
        if !modif.new_content.is_empty() && modif.search_block.is_empty() && modif.search_replace_blocks.is_empty() {
            let content = unescape_llm_output(&modif.new_content);
            fs::write(&full_path, &content).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "File written (full-rewrite mode)");
            continue;
        }

        // ── search_replace_blocks mode (preferred surgical edit) ─────────────
        if !modif.search_replace_blocks.is_empty() {
            let mut current = existing.clone();
            for (i, pair) in modif.search_replace_blocks.iter().enumerate() {
                let search = unescape_llm_output(&pair.search);
                let replace = unescape_llm_output(&pair.replace);
                current = apply_one_search_replace(&modif.file_path, &current, &search, &replace, i)?;
            }
            fs::write(&full_path, current).map_err(|e| e.to_string())?;
            info!(
                path = %modif.file_path,
                count = modif.search_replace_blocks.len(),
                "Patch applied (search_replace_blocks)"
            );
            continue;
        }

        // ── Patch mode ────────────────────────────────────────────────────────
        if modif.search_block.is_empty() {
            return Err(format!(
                "FileModification for '{}' must use target_chunk, new_content, search_replace_blocks, or search_block.",
                modif.file_path
            ));
        }

        let search = unescape_llm_output(&modif.search_block);
        let replace = unescape_llm_output(&modif.replace_block);

        // 1. Exact match
        if existing.contains(&search) {
            let patched = existing.replacen(&search, &replace, 1);
            fs::write(&full_path, patched).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "Patch applied (exact match)");
            continue;
        }

        // 2. CRLF-normalised fallback
        let existing_norm = existing.replace("\r\n", "\n");
        let search_norm = search.replace("\r\n", "\n");
        if existing_norm.contains(&search_norm) {
            let patched = existing_norm.replacen(&search_norm, &replace, 1);
            fs::write(&full_path, patched).map_err(|e| e.to_string())?;
            info!(path = %modif.file_path, "Patch applied (CRLF-normalised)");
            continue;
        }

        // 3. Trim-each-line fallback
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

        // 4. Fuzzy match
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
            "search_block not found in '{}' after exact, CRLF, trim, and fuzzy matching.\n\
             First 120 chars of search_block: {:?}",
            modif.file_path,
            &search.chars().take(120).collect::<String>()
        ));
    }
    Ok(())
}

/// Apply one SEARCH/REPLACE pair to `content`, trying exact → CRLF → trim → fuzzy.
/// Returns the patched string or a descriptive error.
fn apply_one_search_replace(
    file_path: &str,
    content: &str,
    search: &str,
    replace: &str,
    index: usize,
) -> Result<String, String> {
    // 1. Exact
    if content.contains(search) {
        return Ok(content.replacen(search, replace, 1));
    }
    // 2. CRLF-normalised
    let content_norm = content.replace("\r\n", "\n");
    let search_norm = search.replace("\r\n", "\n");
    if content_norm.contains(&search_norm) {
        return Ok(content_norm.replacen(&search_norm, replace, 1));
    }
    // 3. Trim-each-line
    let trim = |s: &str| s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    let content_trim = trim(&content_norm);
    let search_trim = trim(&search_norm);
    if content_trim.contains(&search_trim) {
        return Ok(content_trim.replacen(&search_trim, replace, 1));
    }
    // 4. Fuzzy (whitespace-agnostic)
    if let Some((start, end)) = find_fuzzy_match_range(content, search) {
        return Ok(format!("{}{}{}", &content[..start], replace, &content[end..]));
    }
    Err(format!(
        "search_replace_blocks[{}]: search block not found in '{}' after exact, CRLF, trim, and fuzzy matching.\n\
         First 120 chars of search: {:?}",
        index,
        file_path,
        &search.chars().take(120).collect::<String>()
    ))
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

/// Read a list of relative file paths from the workspace and return their contents.
/// Files larger than 50 000 bytes are skipped with a notice.
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

// ── internal helpers ──────────────────────────────────────────────────────────

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
