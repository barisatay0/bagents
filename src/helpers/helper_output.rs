/// Normalise raw LLM output into a clean JSON string.
///
/// Pipeline:
/// 1. Strip all `<think>…</think>` blocks (reasoning models — qwen3, deepseek-r1, etc.)
/// 2. If bare JSON object, trim trailing text after the final closing brace.
/// 3. Extract JSON from ```json or ``` fences.
/// 4. Repair common truncation patterns (missing closing braces/brackets).
pub fn output(raw: &str) -> String {
    // Step 1: Strip ALL <think> blocks (some models emit more than one)
    let mut s = raw.to_string();
    loop {
        match (s.find("<think>"), s.find("</think>")) {
            (Some(start), Some(end)) if start < end => {
                s = s[..start].to_string() + &s[end + 8..];
            }
            _ => break,
        }
    }
    let trimmed = s.trim().to_string();

    // Step 2: Bare JSON object — strip trailing prose after the closing brace
    if trimmed.starts_with('{') {
        let candidate = match find_json_object_end(&trimmed) {
            Some(end) => trimmed[..end].to_string(),
            None => {
                // No valid closing brace — attempt repair before giving up
                attempt_json_repair(&trimmed)
            }
        };
        return candidate;
    }

    if trimmed.starts_with('[') {
        return trimmed;
    }

    // Step 3: Fenced blocks (```json or ```)
    for fence in &["```json", "```"] {
        if let Some(start) = trimmed.find(fence) {
            let after = &trimmed[start + fence.len()..];
            // Prefer rfind to handle nested fences correctly
            let end = after.rfind("```");
            match end {
                Some(e) => {
                    let content = after[..e].trim().to_string();
                    // Validate — if it starts with { try to find closing brace
                    if content.starts_with('{') {
                        return match find_json_object_end(&content) {
                            Some(pos) => content[..pos].to_string(),
                            None => attempt_json_repair(&content),
                        };
                    }
                    return content;
                }
                // Fence was opened but never closed — model was truncated mid-output
                None => {
                    let content = after.trim().to_string();
                    if content.starts_with('{') {
                        return attempt_json_repair(&content);
                    }
                    return content;
                }
            }
        }
    }

    trimmed
}

/// Best-effort repair of a truncated JSON object.
///
/// LLMs sometimes stop mid-string inside a field. We:
/// 1. Close any open string.
/// 2. Add the minimum closing braces/brackets to make the object syntactically valid.
///
/// This is intentionally conservative — we would rather return something `serde` can
/// partially parse and produce a clear "incomplete JSON" error than return garbage.
fn attempt_json_repair(s: &str) -> String {
    let mut result = s.to_string();

    // Count unclosed braces and brackets (ignoring those inside strings)
    let mut depth_brace: i32 = 0;
    let mut depth_bracket: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let mut last_char_was_comma = false;

    for c in s.chars() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if in_string {
            match c {
                '\\' => escape_next = true,
                '"' => in_string = false,
                _ => {}
            }
            last_char_was_comma = false;
        } else {
            match c {
                '"' => {
                    in_string = true;
                    last_char_was_comma = false;
                }
                '{' => {
                    depth_brace += 1;
                    last_char_was_comma = false;
                }
                '}' => {
                    depth_brace -= 1;
                    last_char_was_comma = false;
                }
                '[' => {
                    depth_bracket += 1;
                    last_char_was_comma = false;
                }
                ']' => {
                    depth_bracket -= 1;
                    last_char_was_comma = false;
                }
                ',' => last_char_was_comma = true,
                _ => {
                    if !c.is_whitespace() {
                        last_char_was_comma = false;
                    }
                }
            }
        }
    }

    // If we ended mid-string, close it
    if in_string {
        result.push('"');
    }

    // Remove trailing comma before closing (invalid JSON)
    let trimmed_result = result.trim_end();
    if last_char_was_comma || trimmed_result.ends_with(',') {
        result = trimmed_result.trim_end_matches(',').to_string();
    }

    // Close open arrays then objects
    for _ in 0..depth_bracket.max(0) {
        result.push(']');
    }
    for _ in 0..depth_brace.max(0) {
        result.push('}');
    }

    result
}

/// Find the byte index immediately after the `}` that closes the top-level JSON object.
/// Uses a simple brace counter — handles nested objects, arrays, and strings.
fn find_json_object_end(s: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for (i, c) in s.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        if in_string {
            match c {
                '\\' => escape_next = true,
                '"' => in_string = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + c.len_utf8());
                    }
                }
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_json_passthrough() {
        assert_eq!(output(r#"{"k":"v"}"#), r#"{"k":"v"}"#);
    }

    #[test]
    fn strips_trailing_prose_after_json() {
        let input = r#"{"k":"v"} Here is my explanation..."#;
        assert_eq!(output(input), r#"{"k":"v"}"#);
    }

    #[test]
    fn strips_single_think_block() {
        let input = "<think>reasoning</think>{\"k\":\"v\"}";
        assert_eq!(output(input), r#"{"k":"v"}"#);
    }

    #[test]
    fn strips_multiple_think_blocks() {
        let input = "<think>a</think><think>b</think>{\"k\":\"v\"}";
        assert_eq!(output(input), r#"{"k":"v"}"#);
    }

    #[test]
    fn strips_json_fence() {
        let input = "```json\n{\"k\":\"v\"}\n```";
        assert_eq!(output(input), r#"{"k":"v"}"#);
    }

    #[test]
    fn strips_plain_fence() {
        let input = "```\n{\"k\":\"v\"}\n```";
        assert_eq!(output(input), r#"{"k":"v"}"#);
    }

    #[test]
    fn repairs_missing_closing_brace() {
        // Simulate a model that stopped before closing the object
        let input = r#"{"k":"v","arr":["a","b"]"#;
        let repaired = output(input);
        // Should be valid JSON after repair
        assert!(serde_json::from_str::<serde_json::Value>(&repaired).is_ok(), "repaired: {}", repaired);
    }

    #[test]
    fn repairs_truncated_mid_string() {
        // Model stopped mid-string — we close the string then the object
        let input = r#"{"k":"this was cut"#;
        let repaired = output(input);
        assert!(serde_json::from_str::<serde_json::Value>(&repaired).is_ok(), "repaired: {}", repaired);
    }

    #[test]
    fn fenced_block_unclosed_fence() {
        // Model emitted ```json but forgot the closing ```
        let input = "```json\n{\"k\":\"v\"}\n";
        let result = output(input);
        assert_eq!(result, r#"{"k":"v"}"#);
    }

    #[test]
    fn nested_objects_preserved() {
        let input = r#"{"a":{"b":{"c":1}}}"#;
        assert_eq!(output(input), input);
    }
}
