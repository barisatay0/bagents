/// Normalise raw LLM output into a clean JSON string.
///
/// Pipeline:
/// 1. Strip all `<think>…</think>` blocks (reasoning models — qwen3, deepseek-r1, etc.)
/// 2. If bare JSON, trim trailing text after the final closing brace.
/// 3. Extract JSON from ```json or ``` fences.
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

    // Step 2: Bare JSON object — also strip trailing prose after the closing brace
    if trimmed.starts_with('{') {
        return match find_json_object_end(&trimmed) {
            Some(end) => trimmed[..end].to_string(),
            None => trimmed,
        };
    }
    if trimmed.starts_with('[') {
        return trimmed;
    }

    // Step 3: Fenced blocks (```json or ```)
    for fence in &["```json", "```"] {
        if let Some(start) = trimmed.find(fence) {
            let after = &trimmed[start + fence.len()..];
            if let Some(end) = after.rfind("```") {
                return after[..end].trim().to_string();
            }
        }
    }

    trimmed
}

/// Find the byte index immediately after the `}` that closes the top-level JSON object.
/// Uses a simple brace counter — handles nested objects and strings.
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
}
