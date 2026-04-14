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
