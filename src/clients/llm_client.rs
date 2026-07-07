use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::helpers::helper_output::output;

const MIN_RESPONSE_CHARS: usize = 50;
const TRUNCATION_FINISH_REASONS: &[&str] = &["length", "max_tokens", "content_filter"];

const ANTHROPIC_CACHE_BETA: &str = "prompt-caching-2024-10-22";

const CACHE_MIN_BYTES: usize = 1_400;

fn apply_patch_tool() -> Value {
    json!({
        "name": "apply_patch",
        "description": "Apply code modifications to the repository. Call this once with all file changes needed to resolve the issue. Use SEARCH/REPLACE blocks for surgical edits — never rewrite entire files unless the file is new or fewer than 30 lines.",
        "input_schema": {
            "type": "object",
            "required": ["thought_process", "branch_name", "files_to_modify"],
            "properties": {
                "thought_process": {
                    "type": "string",
                    "description": "One sentence explaining what was changed and why."
                },
                "branch_name": {
                    "type": "string",
                    "description": "Git branch name, e.g. feature/issue-42-add-auth"
                },
                "files_to_modify": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "required": ["file_path"],
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Relative path inside the workspace, e.g. src/lib.rs"
                            },
                            "new_content": {
                                "type": "string",
                                "description": "Full file content. Use ONLY for new files or files under 30 lines. For existing files use search_replace_blocks instead."
                            },
                            "target_chunk": {
                                "type": "string",
                                "description": "Exact semantic chunk name from SEMANTIC FILE OUTLINES (e.g. function_item:from_env). Use only when replacing a whole named function/struct."
                            },
                            "search_block": {
                                "type": "string",
                                "description": "Legacy search block. Prefer search_replace_blocks array instead."
                            },
                            "replace_block": {
                                "type": "string",
                                "description": "Replacement for search_block."
                            },
                            "search_replace_blocks": {
                                "type": "array",
                                "description": "Ordered list of SEARCH/REPLACE pairs for surgical edits. Preferred over search_block for multiple edits in one file.",
                                "items": {
                                    "type": "object",
                                    "required": ["search", "replace"],
                                    "properties": {
                                        "search": {
                                            "type": "string",
                                            "description": "Exact lines to find — include 2-3 lines of unchanged context above and below the edit site. Must appear verbatim in the file."
                                        },
                                        "replace": {
                                            "type": "string",
                                            "description": "Lines that replace the search block. Write only the changed lines plus the context lines you included in search."
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

pub fn read_file_tool() -> Value {
    json!({
        "name": "read_file",
        "description": "Read the content of a file in the workspace before writing patches. Use this to inspect exact line content, confirm indentation, or understand surrounding context before calling apply_patch. You may call read_file multiple times before calling apply_patch.",
        "input_schema": {
            "type": "object",
            "required": ["file_path"],
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Relative path inside the workspace, e.g. src/config.rs"
                },
                "start_line": {
                    "type": "integer",
                    "description": "1-based line number to start reading from (inclusive). Omit to read from the beginning."
                },
                "end_line": {
                    "type": "integer",
                    "description": "1-based line number to stop reading at (inclusive). Omit to read to the end of the file."
                }
            }
        }
    })
}

fn to_openai_tool(anthropic_tool: Value) -> Value {
    let name = anthropic_tool["name"].clone();
    let description = anthropic_tool["description"].clone();
    let parameters = anthropic_tool["input_schema"].clone();
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters
        }
    })
}

pub enum DevTurnResult {
    ReadFile {
        tool_use_id: String,
        file_path: String,
        start_line: Option<usize>,
        end_line: Option<usize>,
    },
    ApplyPatch(String),
    Error(String),
}

pub async fn ask(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    ask_inner(config, system_prompt, user_prompt, false, None).await
}

#[allow(dead_code)]
pub async fn ask_large(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    ask_inner(config, system_prompt, user_prompt, true, None).await
}

pub async fn ask_large_with_context(
    config: &Config,
    system_prompt: &str,
    static_context: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    ask_inner(config, system_prompt, user_prompt, true, Some(static_context)).await
}

pub async fn ask_dev_turn(
    config: &Config,
    system_prompt: &str,
    conversation: &[Value],
    use_anthropic_cache: bool,
) -> DevTurnResult {
    let client = match Client::builder().timeout(Duration::from_secs(180)).build() {
        Ok(c) => c,
        Err(e) => return DevTurnResult::Error(e.to_string()),
    };

    let max_tokens = config.llm_max_tokens_large;

    let body = build_dev_turn_body(config, system_prompt, conversation, max_tokens, use_anthropic_cache);
    let payload = match serde_json::to_string(&body) {
        Ok(p) => p,
        Err(e) => return DevTurnResult::Error(e.to_string()),
    };

    let mut retries = 0u32;
    let max_retries = 6u32;

    loop {
        let is_anthropic = is_anthropic_endpoint(config);
        let mut req = client
            .post(&config.llm_api_url)
            .header(reqwest::header::CONTENT_TYPE, "application/json");

        if is_anthropic {
            req = req.header("x-api-key", &config.llm_api_key)
                     .header("anthropic-version", "2023-06-01");
        } else {
            req = req.bearer_auth(&config.llm_api_key);
        }

        if use_anthropic_cache {
            req = req.header("anthropic-beta", ANTHROPIC_CACHE_BETA);
        }

        let res = match req.body(payload.clone()).send().await {
            Ok(r) => r,
            Err(e) => return DevTurnResult::Error(e.to_string()),
        };

        let status = res.status();
        let res_text = match res.text().await {
            Ok(t) => t,
            Err(e) => return DevTurnResult::Error(e.to_string()),
        };

        let is_rate_limit = status == StatusCode::TOO_MANY_REQUESTS
            || (!status.is_success()
                && (res_text.to_lowercase().contains("rate limit")
                    || res_text.to_lowercase().contains("try again in")
                    || res_text.to_lowercase().contains("quota exceeded")));

        if is_rate_limit {
            if retries >= max_retries {
                return DevTurnResult::Error(format!(
                    "LLM API rate limit hit {} times — giving up: {}",
                    max_retries, res_text
                ));
            }
            let base_wait = 15u64 * (1u64 << retries.min(4));
            let jitter = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
                % 5) as u64;
            let wait = base_wait + jitter;
            warn!(attempt = retries + 1, max_retries, wait_secs = wait, "Rate limit — backing off");
            sleep(Duration::from_secs(wait)).await;
            retries += 1;
            continue;
        }

        if !status.is_success() {
            return DevTurnResult::Error(format!("LLM API HTTP {}: {}", status, res_text));
        }

        let res_json: Value = match serde_json::from_str(&res_text) {
            Ok(v) => v,
            Err(e) => {
                return DevTurnResult::Error(format!(
                    "LLM API returned non-JSON (status {}): {} — parse err: {}",
                    status,
                    &res_text[..res_text.len().min(200)],
                    e
                ))
            }
        };

        if res_json["error"].is_object() {
            let err_msg = res_json["error"]["message"].as_str().unwrap_or("Unknown API error");
            return DevTurnResult::Error(format!("LLM API Error: {}", err_msg));
        }

        if use_anthropic_cache {
            log_cache_stats(&res_json);
        }

        return extract_dev_turn(&res_json, &res_text);
    }
}

fn build_dev_turn_body(
    config: &Config,
    system_prompt: &str,
    conversation: &[Value],
    max_tokens: u32,
    use_cache: bool,
) -> Value {
    let system = if use_cache {
        json!([{
            "type": "text",
            "text": system_prompt,
            "cache_control": { "type": "ephemeral" }
        }])
    } else {
        json!(system_prompt)
    };

    let tools = json!([read_file_tool(), apply_patch_tool()]);

    if use_cache || is_anthropic_endpoint(config) {
        json!({
            "model": config.llm_model,
            "max_tokens": max_tokens,
            "system": system,
            "messages": conversation,
            "tools": tools,
            "tool_choice": { "type": "any" },
            "temperature": config.llm_temperature
        })
    } else {
        let mut messages = vec![json!({ "role": "system", "content": system_prompt })];
        messages.extend_from_slice(conversation);
        let openai_tools = json!([
            to_openai_tool(read_file_tool()),
            to_openai_tool(apply_patch_tool())
        ]);
        json!({
            "model": config.llm_model,
            "max_tokens": max_tokens,
            "messages": messages,
            "tools": openai_tools,
            "tool_choice": "required",
            "temperature": config.llm_temperature
        })
    }
}

fn extract_dev_turn(res_json: &Value, raw_text: &str) -> DevTurnResult {
    if let Some(content_arr) = res_json["content"].as_array() {
        for block in content_arr {
            if block["type"].as_str() != Some("tool_use") {
                continue;
            }

            let tool_use_id = block["id"].as_str().unwrap_or("").to_string();
            let name = block["name"].as_str().unwrap_or("");

            if name == "read_file" {
                let input = &block["input"];
                let file_path = input["file_path"].as_str().unwrap_or("").to_string();
                let start_line = input["start_line"].as_u64().map(|n| n as usize);
                let end_line = input["end_line"].as_u64().map(|n| n as usize);
                debug!(file = %file_path, start = ?start_line, end = ?end_line, "Agent requested read_file");
                return DevTurnResult::ReadFile { tool_use_id, file_path, start_line, end_line };
            }

            if name == "apply_patch" {
                let serialised = match serde_json::to_string(&block["input"]) {
                    Ok(s) => s,
                    Err(e) => return DevTurnResult::Error(format!("Failed to serialise apply_patch input: {}", e)),
                };
                debug!(chars = serialised.len(), "Extracted apply_patch (Anthropic shape)");
                return DevTurnResult::ApplyPatch(serialised);
            }
        }
    }

    if let Some(tool_calls) = res_json["choices"][0]["message"]["tool_calls"].as_array() {
        for call in tool_calls {
            let name = call["function"]["name"].as_str().unwrap_or("");
            let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");

            if name == "read_file" {
                let tool_use_id = call["id"].as_str().unwrap_or("").to_string();
                let input: Value = serde_json::from_str(args_str).unwrap_or_default();
                let file_path = input["file_path"].as_str().unwrap_or("").to_string();
                let start_line = input["start_line"].as_u64().map(|n| n as usize);
                let end_line = input["end_line"].as_u64().map(|n| n as usize);
                debug!(file = %file_path, "Agent requested read_file (OpenAI shape)");
                return DevTurnResult::ReadFile { tool_use_id, file_path, start_line, end_line };
            }

            if name == "apply_patch" {
                match serde_json::from_str::<Value>(args_str) {
                    Ok(parsed) => {
                        let serialised = serde_json::to_string(&parsed).unwrap_or_default();
                        debug!(chars = serialised.len(), "Extracted apply_patch (OpenAI shape)");
                        return DevTurnResult::ApplyPatch(serialised);
                    }
                    Err(e) => return DevTurnResult::Error(format!("Failed to parse apply_patch args: {}", e)),
                }
            }
        }
    }

    warn!("No tool_use block found — falling back to text extraction");
    let text_content = res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or(raw_text);
    DevTurnResult::ApplyPatch(output(text_content))
}

async fn ask_inner(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
    large_response: bool,
    static_context: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(180))
        .build()?;

    let max_tokens = if large_response {
        config.llm_max_tokens_large
    } else {
        config.llm_max_tokens
    };

    let use_tool_calling = large_response && is_tool_calling_supported(config);
    let use_anthropic_cache = is_anthropic_endpoint(config);

    let body = if use_tool_calling {
        build_anthropic_tool_body(
            config,
            system_prompt,
            user_prompt,
            static_context,
            max_tokens,
            use_anthropic_cache,
        )
    } else if use_anthropic_cache {
        build_anthropic_text_body(
            config,
            system_prompt,
            user_prompt,
            static_context,
            max_tokens,
        )
    } else {
        build_openai_body(
            config,
            system_prompt,
            user_prompt,
            static_context,
            max_tokens,
            use_tool_calling,
        )
    };

    let payload = serde_json::to_string(&body)?;
    debug!(
        model = %config.llm_model,
        max_tokens,
        large_response,
        use_tool_calling,
        use_anthropic_cache,
        "Sending LLM request"
    );

    let mut retries = 0u32;
    let max_retries = 6u32;

    loop {
        let is_anthropic = is_anthropic_endpoint(config);
        let mut req = client
            .post(&config.llm_api_url)
            .header(reqwest::header::CONTENT_TYPE, "application/json");

        if is_anthropic {
            req = req.header("x-api-key", &config.llm_api_key)
                     .header("anthropic-version", "2023-06-01");
        } else {
            req = req.bearer_auth(&config.llm_api_key);
        }

        if use_anthropic_cache {
            req = req.header("anthropic-beta", ANTHROPIC_CACHE_BETA);
        }

        let res = req.body(payload.clone()).send().await?;

        let status = res.status();
        let res_text = res.text().await?;

        let is_rate_limit = status == StatusCode::TOO_MANY_REQUESTS
            || (!status.is_success()
                && (res_text.to_lowercase().contains("rate limit")
                    || res_text.to_lowercase().contains("try again in")
                    || res_text.to_lowercase().contains("quota exceeded")));

        if is_rate_limit {
            if retries >= max_retries {
                return Err(format!(
                    "LLM API rate limit hit {} times — giving up: {}",
                    max_retries, res_text
                )
                .into());
            }
            let base_wait = 15u64 * (1u64 << retries.min(4));
            let jitter = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
                % 5) as u64;
            let wait = base_wait + jitter;
            warn!(
                attempt = retries + 1,
                max_retries,
                wait_secs = wait,
                "Rate limit — backing off"
            );
            sleep(Duration::from_secs(wait)).await;
            retries += 1;
            continue;
        }

        if !status.is_success() {
            return Err(format!("LLM API HTTP {}: {}", status, res_text).into());
        }

        let res_json: Value = serde_json::from_str(&res_text).map_err(|e| {
            format!(
                "LLM API returned non-JSON (status {}): {} — parse err: {}",
                status,
                &res_text[..res_text.len().min(200)],
                e
            )
        })?;

        if res_json["error"].is_object() {
            let err_msg = res_json["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            error!(err = err_msg, "LLM API returned an error object");
            return Err(format!("LLM API Error: {}", err_msg).into());
        }

        if use_anthropic_cache {
            log_cache_stats(&res_json);
        }

        if use_tool_calling {
            return extract_tool_response(&res_json, &res_text, retries, max_retries);
        }

        let raw_content = extract_text_content(&res_json);

        let finish_reason = res_json["choices"][0]["finish_reason"]
            .as_str()
            .or_else(|| res_json["stop_reason"].as_str())
            .unwrap_or("stop")
            .to_string();

        let usage_completion = res_json["usage"]["completion_tokens"]
            .as_u64()
            .or_else(|| res_json["usage"]["output_tokens"].as_u64())
            .unwrap_or(0);

        debug!(
            chars = raw_content.len(),
            finish_reason = %finish_reason,
            completion_tokens = usage_completion,
            "LLM response received"
        );

        if TRUNCATION_FINISH_REASONS.contains(&finish_reason.as_str()) {
            if retries >= max_retries {
                warn!(
                    "Response truncated (finish_reason={}) after {} retries — using partial output",
                    finish_reason, retries
                );
                return Ok(output(&raw_content));
            }
            warn!(
                finish_reason = %finish_reason,
                completion_tokens = usage_completion,
                max_tokens,
                attempt = retries + 1,
                "Response was truncated — retrying with large mode"
            );
            if !large_response {
                return Box::pin(ask_inner(
                    config,
                    system_prompt,
                    user_prompt,
                    true,
                    static_context,
                ))
                .await;
            }
            retries += 1;
            sleep(Duration::from_secs(2)).await;
            continue;
        }

        if raw_content.trim().len() < MIN_RESPONSE_CHARS {
            if retries < max_retries {
                warn!(
                    len = raw_content.trim().len(),
                    attempt = retries + 1,
                    "Response suspiciously short — retrying"
                );
                retries += 1;
                sleep(Duration::from_secs(3)).await;
                continue;
            }
            return Err(format!(
                "LLM returned near-empty response after {} attempts: {:?}",
                max_retries, raw_content
            )
            .into());
        }

        let cleaned = output(&raw_content);

        if let Err(parse_err) = serde_json::from_str::<serde_json::Value>(&cleaned) {
            if retries < max_retries {
                warn!(err = %parse_err, attempt = retries + 1, "Response is not valid JSON — retrying");
                retries += 1;
                sleep(Duration::from_secs(2)).await;
                continue;
            }
            info!(
                "Returning potentially invalid JSON after {} retries",
                max_retries
            );
        }

        return Ok(cleaned);
    }
}

fn build_anthropic_tool_body(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
    static_context: Option<&str>,
    max_tokens: u32,
    use_cache: bool,
) -> Value {
    let system = if use_cache {
        json!([{
            "type": "text",
            "text": system_prompt,
            "cache_control": { "type": "ephemeral" }
        }])
    } else {
        json!(system_prompt)
    };

    let user_content = build_user_content(user_prompt, static_context, use_cache);

    json!({
        "model": config.llm_model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": [
            { "role": "user", "content": user_content }
        ],
        "tools": [apply_patch_tool()],
        "tool_choice": { "type": "any" },
        "temperature": config.llm_temperature
    })
}

fn build_anthropic_text_body(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
    static_context: Option<&str>,
    max_tokens: u32,
) -> Value {
    let system = json!([{
        "type": "text",
        "text": system_prompt,
        "cache_control": { "type": "ephemeral" }
    }]);

    let user_content = build_user_content(user_prompt, static_context, true);

    json!({
        "model": config.llm_model,
        "max_tokens": max_tokens,
        "system": system,
        "messages": [
            { "role": "user", "content": user_content }
        ],
        "temperature": config.llm_temperature
    })
}

fn build_openai_body(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
    static_context: Option<&str>,
    max_tokens: u32,
    use_tool_calling: bool,
) -> Value {
    let full_user = match static_context {
        Some(ctx) if !ctx.is_empty() => format!("{}\n\n{}", ctx, user_prompt),
        _ => user_prompt.to_string(),
    };

    let mut body = if use_tool_calling {
        let openai_tools = json!([to_openai_tool(apply_patch_tool())]);
        json!({
            "model": config.llm_model,
            "max_tokens": max_tokens,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": full_user }
            ],
            "tools": openai_tools,
            "tool_choice": "required",
            "temperature": config.llm_temperature
        })
    } else {
        json!({
            "model": config.llm_model,
            "max_tokens": max_tokens,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": full_user }
            ],
            "temperature": config.llm_temperature
        })
    };

    if !use_tool_calling {
        match config.json_mode.to_lowercase().as_str() {
            "openai" | "groq" | "true" => {
                body["response_format"] = json!({ "type": "json_object" });
            }
            "ollama" => {
                body["format"] = json!("json");
            }
            _ => {}
        }
    }

    body
}

pub fn build_user_content(
    user_prompt: &str,
    static_context: Option<&str>,
    use_cache: bool,
) -> Value {
    match static_context {
        Some(ctx) if !ctx.is_empty() => {
            let should_cache = use_cache && ctx.len() >= CACHE_MIN_BYTES;
            if should_cache {
                json!([
                    {
                        "type": "text",
                        "text": ctx,
                        "cache_control": { "type": "ephemeral" }
                    },
                    {
                        "type": "text",
                        "text": user_prompt
                    }
                ])
            } else {
                json!([{
                    "type": "text",
                    "text": format!("{}\n\n{}", ctx, user_prompt)
                }])
            }
        }
        _ => json!(user_prompt),
    }
}

fn extract_text_content(res_json: &Value) -> String {
    if let Some(content_arr) = res_json["content"].as_array() {
        for block in content_arr {
            if block["type"].as_str() == Some("text")
                && let Some(t) = block["text"].as_str()
            {
                return t.to_string();
            }
        }
    }
    res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn log_cache_stats(res_json: &Value) {
    let usage = &res_json["usage"];
    let read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let written = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    let normal = usage["input_tokens"].as_u64().unwrap_or(0);

    if read > 0 || written > 0 {
        info!(
            cache_read_tokens = read,
            cache_write_tokens = written,
            uncached_input_tokens = normal,
            "Prompt cache stats"
        );
    }
}

pub fn is_anthropic_endpoint(config: &Config) -> bool {
    config.llm_api_url.contains("anthropic.com")
        || config.llm_api_url.contains("api.anthropic")
}

fn extract_tool_response(
    res_json: &Value,
    raw_text: &str,
    _retries: u32,
    _max_retries: u32,
) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(content_arr) = res_json["content"].as_array() {
        for block in content_arr {
            if block["type"].as_str() == Some("tool_use")
                && block["name"].as_str() == Some("apply_patch")
            {
                let input = &block["input"];
                let serialised = serde_json::to_string(input)?;
                debug!(
                    chars = serialised.len(),
                    "Extracted tool_use input (Anthropic shape)"
                );
                return Ok(serialised);
            }
        }
    }

    if let Some(tool_calls) = res_json["choices"][0]["message"]["tool_calls"].as_array() {
        for call in tool_calls {
            if call["function"]["name"].as_str() == Some("apply_patch") {
                let args_str = call["function"]["arguments"].as_str().unwrap_or("{}");
                let parsed: Value = serde_json::from_str(args_str)?;
                let serialised = serde_json::to_string(&parsed)?;
                debug!(
                    chars = serialised.len(),
                    "Extracted tool_call arguments (OpenAI shape)"
                );
                return Ok(serialised);
            }
        }
    }

    warn!("No tool_use block found in response — falling back to text extraction");
    let text_content = res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or(raw_text);
    Ok(output(text_content))
}

pub fn is_tool_calling_supported(config: &Config) -> bool {
    let mode = config.json_mode.to_lowercase();
    if mode == "none" {
        return false;
    }
    if mode == "ollama" {
        return false;
    }
    true
}
