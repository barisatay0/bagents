use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::helpers::helper_output::output;

const MIN_RESPONSE_CHARS: usize = 50;
const TRUNCATION_FINISH_REASONS: &[&str] = &["length", "max_tokens", "content_filter"];

// Anthropic beta header value that enables prompt caching.
// Ignored automatically by non-Anthropic endpoints because they never check it.
const ANTHROPIC_CACHE_BETA: &str = "prompt-caching-2024-10-22";

// Anthropic requires a cached block to be at least 1 024 tokens.
// We only attach cache_control when the text exceeds this conservative byte
// threshold (1 char ≈ 0.75 tokens on average; 1 400 bytes ≈ 1 050 tokens).
const CACHE_MIN_BYTES: usize = 1_400;

/// The tool definition sent to the LLM for developer agent requests.
/// The schema mirrors `DeveloperResponse` exactly so the orchestrator needs
/// no changes — we just deserialise `tool_use.input` instead of text content.
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
                                            "description": "Exact lines to find — include 2-3 lines of unchanged context above and below the edit. Must appear verbatim in the file."
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

// ── public API ────────────────────────────────────────────────────────────────

pub async fn ask(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    ask_inner(config, system_prompt, user_prompt, false, None).await
}

pub async fn ask_large(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    ask_inner(config, system_prompt, user_prompt, true, None).await
}

/// Like `ask_large` but accepts an optional large static context string (e.g.
/// the repository tree + file contents) that should be prompt-cached on
/// Anthropic endpoints.  On non-Anthropic endpoints the string is simply
/// appended to the user prompt with no cache annotation.
pub async fn ask_large_with_context(
    config: &Config,
    system_prompt: &str,
    static_context: &str,
    user_prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    ask_inner(config, system_prompt, user_prompt, true, Some(static_context)).await
}

// ── core implementation ───────────────────────────────────────────────────────

async fn ask_inner(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
    large_response: bool,
    // Optional static context to cache (repo tree, file contents, etc.)
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

    // Developer agent requests use tool calling; planner/reviewer use JSON mode.
    let use_tool_calling = large_response && is_tool_calling_supported(config);
    let use_anthropic_cache = is_anthropic_endpoint(config);

    // ── Build request body ────────────────────────────────────────────────────

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

    // ── Retry loop ────────────────────────────────────────────────────────────

    let mut retries = 0u32;
    let max_retries = 6u32;

    loop {
        let mut req = client
            .post(&config.llm_api_url)
            .bearer_auth(&config.llm_api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json");

        // Attach the prompt-caching beta header for Anthropic endpoints.
        if use_anthropic_cache {
            req = req.header("anthropic-beta", ANTHROPIC_CACHE_BETA);
        }

        let res = req.body(payload.clone()).send().await?;

        let status = res.status();
        let res_text = res.text().await?;

        // ── Rate-limit detection ──────────────────────────────────────────────
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
                .subsec_millis()
                % 5000) as u64
                / 1000;
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

        // Log cache usage when available (Anthropic includes this in usage).
        if use_anthropic_cache {
            log_cache_stats(&res_json);
        }

        // ── Tool-calling response extraction ──────────────────────────────────
        if use_tool_calling {
            return extract_tool_response(&res_json, &res_text, retries, max_retries);
        }

        // ── Standard text response extraction ─────────────────────────────────
        // Anthropic text responses use `content[0].text`; OpenAI uses
        // `choices[0].message.content`.  Try both shapes.
        let raw_content = extract_text_content(&res_json);

        let finish_reason = res_json["choices"][0]["finish_reason"]
            .as_str()
            // Anthropic uses `stop_reason` at the top level
            .or_else(|| res_json["stop_reason"].as_str())
            .unwrap_or("stop")
            .to_string();

        let usage_completion = res_json["usage"]["completion_tokens"]
            .as_u64()
            // Anthropic calls this `output_tokens`
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

// ── body builders ─────────────────────────────────────────────────────────────

/// Build an Anthropic-native request body with `cache_control` breakpoints on:
///   1. The system prompt (always static across all calls for a given agent role).
///   2. The static context block (repo tree + file contents), when provided and
///      large enough to exceed Anthropic's minimum cacheable token count.
///
/// The dynamic user prompt (issue text + feedback) is always sent uncached so
/// that changes between dev-loop attempts are reflected immediately.
fn build_anthropic_tool_body(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
    static_context: Option<&str>,
    max_tokens: u32,
    use_cache: bool,
) -> Value {
    // System block — always cache when using Anthropic.
    let system = if use_cache {
        json!([{
            "type": "text",
            "text": system_prompt,
            "cache_control": { "type": "ephemeral" }
        }])
    } else {
        json!(system_prompt)
    };

    // User message — split into [cached static context] + [dynamic prompt].
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

/// Build an Anthropic-native request body for planner / reviewer (text mode).
/// Adds `cache_control` on the system prompt and, if large enough, the static
/// context block.  A `response_format` field is NOT included — Anthropic's API
/// doesn't support it; JSON output is enforced via the system prompt instead.
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

/// Build an OpenAI-compatible request body.  No `cache_control` fields are
/// added; OpenAI's automatic prompt caching works transparently on their end
/// without any request-side annotation.
fn build_openai_body(
    config: &Config,
    system_prompt: &str,
    user_prompt: &str,
    static_context: Option<&str>,
    max_tokens: u32,
    use_tool_calling: bool,
) -> Value {
    // Merge static context into the user turn (flat string, no annotations).
    let full_user = match static_context {
        Some(ctx) if !ctx.is_empty() => format!("{}\n\n{}", ctx, user_prompt),
        _ => user_prompt.to_string(),
    };

    let mut body = if use_tool_calling {
        json!({
            "model": config.llm_model,
            "max_tokens": max_tokens,
            "system": system_prompt,
            "messages": [
                { "role": "user", "content": full_user }
            ],
            "tools": [apply_patch_tool()],
            "tool_choice": { "type": "any" },
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

    // JSON mode for planner / reviewer on OpenAI-compatible endpoints.
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

// ── helpers ───────────────────────────────────────────────────────────────────

/// Construct the `content` array for the user turn.
///
/// When `use_cache` is true and a static context is provided with enough bytes,
/// the context block is annotated with `cache_control` so Anthropic can cache
/// it independently of the dynamic user prompt that follows.
///
/// Layout:
///   [cached static context block]  ← cache_control: ephemeral  (if large enough)
///   [dynamic user prompt block]    ← no cache annotation
fn build_user_content(
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
                // Context below threshold or caching disabled: merge into one block.
                json!([{
                    "type": "text",
                    "text": format!("{}\n\n{}", ctx, user_prompt)
                }])
            }
        }
        _ => json!(user_prompt),
    }
}

/// Extract the text content from either an Anthropic or OpenAI response shape.
fn extract_text_content(res_json: &Value) -> String {
    // Anthropic: { "content": [{ "type": "text", "text": "..." }] }
    if let Some(content_arr) = res_json["content"].as_array() {
        for block in content_arr {
            if block["type"].as_str() == Some("text") {
                if let Some(t) = block["text"].as_str() {
                    return t.to_string();
                }
            }
        }
    }
    // OpenAI: { "choices": [{ "message": { "content": "..." } }] }
    res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// Log Anthropic prompt-caching statistics when present in the response.
/// These fields are only present on Anthropic API responses; nothing happens
/// for OpenAI/Groq because those fields will simply be null.
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

/// Returns true when the configured endpoint looks like the Anthropic API.
/// We use this to decide whether to:
///   (a) send an `anthropic-beta` header, and
///   (b) use Anthropic's content-block message format with `cache_control`.
fn is_anthropic_endpoint(config: &Config) -> bool {
    config.llm_api_url.contains("anthropic.com")
        || config.llm_api_url.contains("api.anthropic")
}

/// Extract and serialise the `apply_patch` tool call input as a JSON string
/// so the caller (`orchestrator.rs`) can deserialise it into `DeveloperResponse`
/// without any changes.
///
/// Supports both Anthropic-style (`content[].type == "tool_use"`) and
/// OpenAI-style (`choices[0].message.tool_calls[0]`) response shapes so the
/// same code works with Groq, OpenAI, and Claude API endpoints.
fn extract_tool_response(
    res_json: &Value,
    raw_text: &str,
    retries: u32,
    max_retries: u32,
) -> Result<String, Box<dyn std::error::Error>> {
    // ── Anthropic API shape ────────────────────────────────────────────────
    // {"content": [{"type": "tool_use", "name": "apply_patch", "input": {...}}]}
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

    // ── OpenAI / Groq API shape ───────────────────────────────────────────
    // {"choices":[{"message":{"tool_calls":[{"function":{"name":"apply_patch","arguments":"{...}"}}]}}]}
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

    // ── Fallback: model emitted text despite tool_choice: any ─────────────
    // Some providers (older Ollama, misc proxies) ignore tool_choice. Fall
    // back to text extraction so the orchestrator still gets something to work with.
    warn!("No tool_use block found in response — falling back to text extraction");
    if retries < max_retries {
        // The orchestrator will handle the parse error and retry with feedback.
    }
    let text_content = res_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or(raw_text);
    Ok(output(text_content))
}

/// Returns true when the configured provider is known to support tool calling
/// well enough to rely on it. Ollama support is patchy, so we disable it there.
fn is_tool_calling_supported(config: &Config) -> bool {
    let mode = config.json_mode.to_lowercase();
    // "none" means the user explicitly disabled structured output features.
    if mode == "none" {
        return false;
    }
    // Ollama's tool-calling support is model-dependent and often unreliable.
    if mode == "ollama" {
        return false;
    }
    true
}
