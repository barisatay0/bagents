/// Normalise raw LLM output into a clean JSON string.
///
/// Handles three common LLM response formats:
/// 1. Bare JSON (starts with `{` or `[`) — returned as-is.
/// 2. Fenced with ` ```json … ``` ` — content between fences is extracted.
/// 3. Fenced with plain ` ``` … ``` ` — same extraction, no language specifier.
pub fn output(raw: &str) -> String {
    let trimmed = raw.trim();

    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return trimmed.to_string();
    }

    // Try ```json fence first, then plain ``` fence
    for fence in &["```json", "```"] {
        if let Some(start) = trimmed.find(fence) {
            let after = &trimmed[start + fence.len()..];
            if let Some(end) = after.rfind("```") {
                return after[..end].trim().to_string();
            }
        }
    }

    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_json_passthrough() {
        let input = r#"{"key": "value"}"#;
        assert_eq!(output(input), input);
    }

    #[test]
    fn strips_json_fence() {
        let input = "```json\n{\"key\": \"value\"}\n```";
        assert_eq!(output(input), r#"{"key": "value"}"#);
    }

    #[test]
    fn strips_plain_fence() {
        let input = "```\n{\"key\": \"value\"}\n```";
        assert_eq!(output(input), r#"{"key": "value"}"#);
    }

    #[test]
    fn returns_trimmed_fallback() {
        let input = "   some text   ";
        assert_eq!(output(input), "some text");
    }
}
