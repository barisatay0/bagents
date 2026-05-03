use std::path::Path;
use tree_sitter::{Language, Parser, Query, QueryCursor};

#[derive(Debug, Clone)]
pub struct CodeChunk {
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
}

/// A lightweight symbol entry used for repository map output.
/// Carries only the information needed to understand a file's public API —
/// name, kind, signature line — without the full body content.
#[derive(Debug, Clone)]
pub struct SymbolSignature {
    /// Display name of the symbol (function name, struct name, etc.)
    pub name: String,
    /// Tree-sitter node kind (e.g. "function_item", "struct_item")
    pub kind: String,
    /// The first line of the declaration — just the signature, not the body.
    pub signature: String,
    /// 1-based line number of the declaration.
    pub line: usize,
}

fn get_language_and_query(extension: &str) -> Option<(Language, &'static str)> {
    match extension {
        "rs" => {
            let query = "
                (function_item name: (identifier) @name) @block
                (struct_item name: (type_identifier) @name) @block
                (impl_item type: (type_identifier) @name) @block
            ";
            Some((tree_sitter_rust::language(), query))
        }
        "js" | "jsx" => {
            let query = "
                (function_declaration name: (identifier) @name) @block
                (class_declaration name: (identifier) @name) @block
            ";
            Some((tree_sitter_javascript::language(), query))
        }
        "py" => {
            let query = "
                (function_definition name: (identifier) @name) @block
                (class_definition name: (identifier) @name) @block
            ";
            Some((tree_sitter_python::language(), query))
        }
        "ts" => {
            let query = "
                (function_declaration name: (identifier) @name) @block
                (class_declaration name: (identifier) @name) @block
                (method_definition name: (property_identifier) @name) @block
                (interface_declaration name: (type_identifier) @name) @block
                (type_alias_declaration name: (type_identifier) @name) @block
            ";
            Some((tree_sitter_typescript::language_typescript(), query))
        }
        "tsx" => {
            let query = "
                (function_declaration name: (identifier) @name) @block
                (class_declaration name: (identifier) @name) @block
                (method_definition name: (property_identifier) @name) @block
                (interface_declaration name: (type_identifier) @name) @block
                (lexical_declaration (variable_declarator name: (identifier) @name value: (arrow_function))) @block
            ";
            Some((tree_sitter_typescript::language_tsx(), query))
        }
        _ => None,
    }
}

// ── Full chunk extraction (used by developer agent) ───────────────────────────

pub fn extract_semantic_chunks(file_path: &str, source_code: &str) -> Vec<CodeChunk> {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let (language, query_str) = match get_language_and_query(extension) {
        Some(lq) => lq,
        None => return vec![],
    };

    let mut parser = Parser::new();
    parser
        .set_language(language)
        .expect("Error loading grammar");

    let tree = parser
        .parse(source_code, None)
        .expect("Error parsing source code");
    let query = Query::new(language, query_str).expect("Error compiling query");
    let mut cursor = QueryCursor::new();

    let matches = cursor.matches(&query, tree.root_node(), source_code.as_bytes());
    let mut chunks = Vec::new();

    for m in matches {
        let mut name = String::new();
        let mut block_node = None;

        for capture in m.captures {
            let capture_name = query.capture_names()[capture.index as usize].as_str();

            if capture_name == "name" {
                name = capture
                    .node
                    .utf8_text(source_code.as_bytes())
                    .unwrap()
                    .to_string();
            } else if capture_name == "block" {
                block_node = Some(capture.node);
            }
        }

        if let Some(node) = block_node {
            let content = node.utf8_text(source_code.as_bytes()).unwrap().to_string();
            let kind_str = node.kind().to_string();
            let unique_name = format!("{}:{}", kind_str, name);

            chunks.push(CodeChunk {
                name: unique_name,
                kind: kind_str,
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                content,
            });
        }
    }

    chunks
}

// ── Signature-only extraction (used by repository map) ───────────────────────

/// Extract symbol signatures from a source file without including body content.
///
/// For each matching symbol we return only:
///   • the symbol name
///   • its kind (function, struct, class, etc.)
///   • the first line of the declaration (the signature line)
///   • the 1-based start line number
///
/// This keeps the repository map compact: a 2 000-line file might export 30
/// functions but the map entry is just 30 one-liners instead of 2 000 lines of
/// source.
pub fn extract_signatures(file_path: &str, source_code: &str) -> Vec<SymbolSignature> {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let (language, query_str) = match get_language_and_query(extension) {
        Some(lq) => lq,
        None => return vec![],
    };

    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        return vec![];
    }

    let tree = match parser.parse(source_code, None) {
        Some(t) => t,
        None => return vec![],
    };

    let query = match Query::new(language, query_str) {
        Ok(q) => q,
        Err(_) => return vec![],
    };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&query, tree.root_node(), source_code.as_bytes());
    let source_lines: Vec<&str> = source_code.lines().collect();

    let mut signatures = Vec::new();

    for m in matches {
        let mut name = String::new();
        let mut block_node = None;

        for capture in m.captures {
            let capture_name = query.capture_names()[capture.index as usize].as_str();
            if capture_name == "name" {
                name = capture
                    .node
                    .utf8_text(source_code.as_bytes())
                    .unwrap_or("")
                    .to_string();
            } else if capture_name == "block" {
                block_node = Some(capture.node);
            }
        }

        if let Some(node) = block_node {
            let start_row = node.start_position().row; // 0-based
            let kind_str = node.kind().to_string();

            // Grab the first non-empty line of the declaration.  For multi-line
            // signatures (e.g. Rust `pub fn foo(\n    x: i32,\n) -> Bar`) we
            // collect lines until we hit the opening brace `{` or `:` (Python)
            // so the team lead can see parameter types without the full body.
            let sig = extract_signature_lines(&source_lines, start_row, &kind_str);

            signatures.push(SymbolSignature {
                name,
                kind: kind_str,
                signature: sig,
                line: start_row + 1,
            });
        }
    }

    signatures
}

/// Collect the declaration signature starting at `start_row`.
///
/// We read lines until we detect the opening of the body (`{` at the end of a
/// line for brace-languages, `:` for Python defs, or a hard cap of 4 lines) so
/// we never accidentally include any body content.
fn extract_signature_lines(source_lines: &[&str], start_row: usize, kind: &str) -> String {
    let is_python_def = kind.contains("function_definition") || kind.contains("class_definition");

    let mut parts: Vec<&str> = Vec::new();
    let cap = (start_row + 4).min(source_lines.len());

    for row in start_row..cap {
        let line = source_lines[row].trim_end();
        parts.push(line);

        if is_python_def {
            // Python signatures end at the colon that closes the `def`/`class` line.
            if line.trim_end().ends_with(':') {
                break;
            }
        } else {
            // For brace-languages: stop when we see the opening `{` that begins
            // the body.  We trim to avoid false positives from trailing comments.
            let trimmed = line.trim();
            if trimmed.ends_with('{') || trimmed == "{" {
                break;
            }
            // Also stop at a line that IS only the opening brace (the next line).
            if trimmed == "where" {
                // Rust `where` clause — keep reading until `{`.
                continue;
            }
        }
    }

    parts.join("\n")
}

/// Format a file's signatures as a compact block suitable for the repository map.
///
/// Example output for `src/config.rs`:
///
/// ```text
/// src/config.rs
///   struct_item   Config                    (line 12)
///   impl_item     Config                    (line 28)
///   function_item Config::from_env          (line 30)
/// ```
pub fn format_file_signatures(file_path: &str, signatures: &[SymbolSignature]) -> String {
    if signatures.is_empty() {
        return String::new();
    }

    let mut out = format!("{}:\n", file_path);
    for sig in signatures {
        // Indent the signature text for readability; replace inner newlines with
        // a space so multi-line signatures stay on one logical line in the map.
        let sig_oneline = sig.signature.split('\n').collect::<Vec<_>>().join(" ");
        out.push_str(&format!(
            "  [{kind}] {sig}  (line {line})\n",
            kind = sig.kind,
            sig = sig_oneline,
            line = sig.line,
        ));
    }
    out
}
