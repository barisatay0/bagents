pub fn output(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return trimmed.to_string();
    }

    if let Some(start) = raw.find("```json") {
        let content_after_start = &raw[start + 7..];
        if let Some(end) = content_after_start.rfind("```") {
            return content_after_start[..end].trim().to_string();
        }
    }

    trimmed.to_string()
}
