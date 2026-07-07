use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Serialize, Deserialize};

use crate::clients::llm_client::{
    self, DevTurnResult, build_user_content, is_anthropic_endpoint, is_tool_calling_supported,
};
use crate::config::Config;
use crate::models::{
    developer_response::DeveloperResponse, reviewer_response::ReviewerResponse,
    teamlead_response::TeamLeaderResponse, issue::Issue,
};
use crate::prompts::Prompts;
use crate::services::{file_system, git_local, build_tracker, build_repo_service, IssueTracker, RepoService};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to listen for SIGINT");
        let mut sigterm = signal(SignalKind::terminate()).expect("failed to listen for SIGTERM");
        tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), starting graceful shutdown...");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, starting graceful shutdown...");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("failed to listen for Ctrl+C");
        info!("Received Ctrl+C, starting graceful shutdown...");
    }
    SHUTDOWN.store(true, Ordering::SeqCst);
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct IssueState {
    pub id: String,
    pub status: String,
    pub failure_count: u32,
    pub last_attempted_at: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct AppState {
    pub issues: std::collections::HashMap<String, IssueState>,
}

impl AppState {
    fn load() -> Self {
        let path = std::env::current_dir()
            .unwrap_or_default()
            .join("bagents_state.json");
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn save(&self) {
        let path = std::env::current_dir()
            .unwrap_or_default()
            .join("bagents_state.json");
        if let Ok(content) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, content);
        }
    }
}

const PLACEHOLDER_MARKERS: &[&str] = &[
    "// TODO", "// FIXME", "// HACK", "// XXX", "todo!()", "unimplemented!()",
    "... existing code ...", "/* TODO */", "# TODO", "FIXME", "// ...",
    "{/* ... */}", "// rest of", "// ... rest", "/* rest of",
];

const MAX_FILES_PER_CYCLE: usize = 2;
const MAX_READ_FILE_TURNS: usize = 8;

pub async fn run_factory(
    config: &Config,
    prompts: &Prompts,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Factory started — polling for issues continuously");

    // Spawn the graceful shutdown handler
    tokio::spawn(shutdown_signal());

    let tracker = match build_tracker(config) {
        Ok(t) => t,
        Err(e) => {
            error!(err = %e, "Could not build issue tracker");
            return Err(e);
        }
    };

    let repo_service = match build_repo_service(config) {
        Ok(r) => r,
        Err(e) => {
            error!(err = %e, "Could not build repo service");
            return Err(e);
        }
    };

    if config.tracker_type != config.repo_type {
        info!(
            tracker = %config.tracker_type,
            repo = %config.repo_type,
            "Mixed service configuration active (this is supported and valid)"
        );
    }

    let mut state = AppState::load();

    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("Graceful shutdown requested. Exiting factory.");
            break;
        }

        info!("Checking for open issues...");

        let issues = match tracker.fetch_open_issues().await {
            Ok(i) => i,
            Err(e) => {
                error!(err = %e, "Tracker API error — retrying in {}s", config.error_retry_secs);
                for _ in 0..config.error_retry_secs {
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                }
                continue;
            }
        };

        let target_issue = match issues
            .into_iter()
            .find(|i| {
                if let Some(istate) = state.issues.get(&i.id) {
                    if istate.status == "success" {
                        false
                    } else if istate.failure_count >= 3 {
                        false
                    } else {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        now >= istate.last_attempted_at + 900
                    }
                } else {
                    true
                }
            })
        {
            Some(i) => i,
            None => {
                info!("No new issues — resting for {}s", config.poll_interval_secs);
                for _ in 0..config.poll_interval_secs {
                    if SHUTDOWN.load(Ordering::SeqCst) {
                        break;
                    }
                    sleep(Duration::from_secs(1)).await;
                }
                continue;
            }
        };

        if SHUTDOWN.load(Ordering::SeqCst) {
            info!("Graceful shutdown requested. Exiting factory before processing issue.");
            break;
        }

        info!(
            issue = %target_issue.id,
            title = %target_issue.title,
            "Processing issue"
        );

        if let Err(e) = git_local::reset_to_base(config) {
            error!(err = %e, "Could not reset to base branch — skipping issue");
            for _ in 0..10 {
                if SHUTDOWN.load(Ordering::SeqCst) {
                    break;
                }
                sleep(Duration::from_secs(1)).await;
            }
            continue;
        }

        let is_successful = process_issue(config, prompts, &target_issue, &*tracker, &*repo_service).await;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let istate = state.issues.entry(target_issue.id.clone()).or_insert_with(|| IssueState {
            id: target_issue.id.clone(),
            status: String::new(),
            failure_count: 0,
            last_attempted_at: 0,
        });

        istate.last_attempted_at = now;

        if is_successful {
            info!(issue = %target_issue.id, "Issue completed successfully");
            istate.status = "success".to_string();
            state.save();
        } else {
            istate.failure_count += 1;
            istate.status = "failed".to_string();
            warn!(
                issue = %target_issue.id,
                failures = istate.failure_count,
                "Issue failed, exhausted, or deferred"
            );
            state.save();
        }

        for _ in 0..10 {
            if SHUTDOWN.load(Ordering::SeqCst) {
                break;
            }
            sleep(Duration::from_secs(1)).await;
        }
    }
    Ok(())
}

async fn plan_issue(
    config: &Config,
    prompts: &Prompts,
    issue_text: &str,
    repo_map: &str,
) -> Option<TeamLeaderResponse> {
    info!("Team leader is planning...");

    let user_prompt = format!(
        "Issue to resolve:\n\
         {issue_text}\n\n\
         Use the REPOSITORY MAP above to identify which files need changing. \
         Only request full file contents (files_to_read) for the files you are \
         confident need modification — the map already shows you all available \
         symbols and their signatures."
    );

    let raw = match llm_client::ask_large_with_context(
        config,
        &prompts.team_lead,
        repo_map,
        &user_prompt,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(err = %e, "LLM request failed during planning");
            return None;
        }
    };

    match serde_json::from_str::<TeamLeaderResponse>(&raw) {
        Ok(r) => Some(r),
        Err(e) => {
            error!(err = %e, raw = %raw, "Team lead returned invalid JSON");
            None
        }
    }
}

struct TokenBudgetResult {
    lead_res: TeamLeaderResponse,
    remaining_files: Vec<String>,
    was_truncated: bool,
}

fn apply_token_budget(mut lead_res: TeamLeaderResponse) -> TokenBudgetResult {
    if lead_res.files_to_read.len() <= MAX_FILES_PER_CYCLE {
        return TokenBudgetResult {
            lead_res,
            remaining_files: Vec::new(),
            was_truncated: false,
        };
    }

    let remaining_files = lead_res.files_to_read.split_off(MAX_FILES_PER_CYCLE);

    warn!(
        retained = lead_res.files_to_read.len(),
        deferred = remaining_files.len(),
        "Token budget active — deferring files to next cycle"
    );

    lead_res.architectural_plan = format!(
        "{} [SYSTEM OVERRIDE: Due to token limits, ONLY modify these specific files in this PR: {:?}. Ignore the rest for now.]",
        lead_res.architectural_plan, lead_res.files_to_read
    );

    TokenBudgetResult {
        lead_res,
        remaining_files,
        was_truncated: true,
    }
}

struct DevLoopResult {
    success: bool,
    branch_name: String,
    thought_process: String,
}

async fn execute_dev_loop(
    config: &Config,
    prompts: &Prompts,
    issue_text: &str,
    repo_tree: &str,
    lead_res: &TeamLeaderResponse,
    branch_name: &str,
) -> DevLoopResult {
    let max_attempts = 6;
    let mut feedback_history = String::new();
    let mut consecutive_json_failures: u32 = 0;

    let specific_files = if lead_res.chunks_to_read.is_empty() {
        file_system::read_specific_files(config, lead_res.files_to_read.clone())
    } else {
        let mut chunks_output = String::new();
        for file in &lead_res.files_to_read {
            chunks_output.push_str(&file_system::read_specific_chunks(
                config,
                file,
                lead_res.chunks_to_read.clone(),
            ));
        }
        chunks_output
    };

    let semantic_outlines =
        file_system::read_semantic_outlines(config, lead_res.files_to_read.clone());

    let static_context = format!(
        "Project: '{project}'.\n\
         \n\
         {repo_tree}\n\
         \n\
         {semantic_outlines}\n\
         \n\
         {specific_files}",
        project = config.tracker_project,
        repo_tree = repo_tree,
        semantic_outlines = semantic_outlines,
        specific_files = specific_files,
    );

    let dev_prompt = prompts.for_agent(&lead_res.assigned_agent);
    let use_anthropic_cache = is_anthropic_endpoint(config);
    let supports_tools = is_tool_calling_supported(config);

    for attempt in 1..=max_attempts {
        info!(attempt, max_attempts, agent = %lead_res.assigned_agent, "Developer writing code");

        if let Err(e) = git_local::reset_working_tree(config) {
            error!(err = %e, "Could not reset working tree before attempt");
            break;
        }

        let dynamic_prompt = build_dynamic_dev_prompt(
            issue_text,
            lead_res,
            &feedback_history,
            attempt,
            max_attempts,
        );

        let initial_user_content = if supports_tools && use_anthropic_cache {
            build_user_content(&dynamic_prompt, Some(&static_context), true)
        } else {
            serde_json::json!(format!("{}\n\n{}", static_context, dynamic_prompt))
        };

        let mut conversation: Vec<serde_json::Value> = vec![
            serde_json::json!({ "role": "user", "content": initial_user_content }),
        ];

        let raw = if supports_tools {
            run_dev_tool_loop(
                config,
                dev_prompt,
                &mut conversation,
                use_anthropic_cache,
                attempt,
            )
            .await
        } else {
            match llm_client::ask_large_with_context(
                config,
                dev_prompt,
                &static_context,
                &dynamic_prompt,
            )
            .await
            {
                Ok(r) => Ok(r),
                Err(e) => Err(e.to_string()),
            }
        };

        let raw = match raw {
            Ok(r) => r,
            Err(e) => {
                error!(err = %e, "LLM request failed during dev loop");
                feedback_history = format!(
                    "SYSTEM ERROR: LLM request failed with: {}. \
                     This was not your fault. On your next attempt, output a complete, valid JSON object.",
                    e
                );
                continue;
            }
        };

        let dev_res: DeveloperResponse = match serde_json::from_str(&raw) {
            Ok(r) => {
                consecutive_json_failures = 0;
                r
            }
            Err(e) => {
                consecutive_json_failures += 1;
                warn!(err = %e, consecutive = consecutive_json_failures, "Developer returned invalid JSON — retrying");

                let snippet = raw.chars().take(400).collect::<String>();
                let tail = if raw.len() > 200 {
                    raw.chars().rev().take(200).collect::<String>().chars().rev().collect::<String>()
                } else {
                    String::new()
                };

                feedback_history = if consecutive_json_failures >= 2 {
                    format!(
                        "CRITICAL: Your response was invalid JSON for {} attempts in a row.\n\
                         serde error: '{}'\n\
                         Response start: {:?}\n\
                         Response end: {:?}\n\
                         YOU MUST output ONLY this exact structure (no prose, no markdown, no thinking text):\n\
                         {{\"thought_process\":\"...\",\"branch_name\":\"feature/...\",\"files_to_modify\":[{{\"file_path\":\"src/...\",\"target_chunk\":\"...\",\"new_content\":\"...\"}}]}}",
                        consecutive_json_failures, e, snippet, tail
                    )
                } else {
                    format!(
                        "CRITICAL: Your last response was NOT valid JSON.\n\
                         serde error: '{}'\n\
                         Your response started with: {:?}\n\
                         Your response ended with: {:?}\n\
                         Rules:\n\
                         1. Output ONLY a raw JSON object — zero text before or after the {{}}.\n\
                         2. Never use real newlines inside JSON string values — use \\n.\n\
                         3. Never use unescaped quotes inside strings — use \\\".\n\
                         4. The JSON must be COMPLETE — do not stop mid-object.",
                        e, snippet, tail
                    )
                };
                continue;
            }
        };

        info!(thought = %dev_res.thought_process, "Developer response parsed");

        if dev_res.files_to_modify.is_empty() {
            warn!("Developer returned zero file modifications — retrying");
            feedback_history = format!(
                "CRITICAL: Your response contained an empty `files_to_modify` array.\n\
                 You MUST include at least one file modification.\n\
                 The issue requires changes to: {:?}",
                lead_res.files_to_read
            );
            continue;
        }

        if let Some((bad_file, bad_marker)) = find_placeholder(&dev_res) {
            warn!(file = %bad_file, marker = %bad_marker, "Placeholder detected — rejecting");
            feedback_history = format!(
                "CRITICAL ERROR: File '{}' contains the placeholder '{}'. \
                 Placeholders are STRICTLY FORBIDDEN. \
                 You MUST write the COMPLETE, production-ready implementation. \
                 Every function body, every field, every line of logic — fully written out. \
                 Do not use shortcuts like '// ...', '// existing code', or '// TODO'.",
                bad_file, bad_marker
            );
            continue;
        }

        if let Err(e) = file_system::apply_modifications(config, dev_res.files_to_modify.clone()) {
            warn!(err = %e, "Patch failed");
            feedback_history = format!(
                "CRITICAL ERROR: Failed to apply your patch. Details:\n{}\n\n\
                 Instructions:\n\
                 - If using `target_chunk`: the name must EXACTLY match one from the 'Available chunks' list shown in SEMANTIC FILE OUTLINES.\n\
                 - If using `search_block`: copy the block CHARACTER-FOR-CHARACTER from the file content. Include 3+ lines of context.\n\
                 - Do NOT invent chunk names or approximate search blocks.\n\
                 - TIP: Use the `read_file` tool first to inspect the exact content of the file before writing your patch.",
                e
            );
            continue;
        }

        if let Err(verify_err) = git_local::run_verification(config) {
            let is_relevant_error = dev_res.files_to_modify.iter().any(|m| {
                if m.file_path.is_empty() {
                    return false;
                }
                let file_name = std::path::Path::new(&m.file_path)
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                verify_err.contains(&m.file_path) || (!file_name.is_empty() && verify_err.contains(file_name.as_ref()))
            });

            if is_relevant_error {
                warn!(attempt, "Verification failed on modified files");
                feedback_history = format!(
                    "CRITICAL VERIFICATION ERROR: Your code failed the build/linter.\n\
                     Fix ONLY the errors in the files you modified. Do not change anything else.\n\
                     Diagnostic output:\n{}",
                    verify_err
                );
                continue;
            } else {
                warn!(
                    attempt,
                    "Verification failed but errors are in out-of-scope files — bypassing"
                );
            }
        }

        let commit_msg = format!(
            "feat: Resolve issue #{} (attempt {})\n\n{}",
            branch_name.trim_start_matches("feature/issue-"),
            attempt,
            truncate_to(&dev_res.thought_process, 500)
        );
        if let Err(e) = git_local::commit_changes(config, &commit_msg) {
            warn!(err = %e, "Git commit failed — retrying dev turn");
            feedback_history = format!("CRITICAL: git commit failed: {}", e);
            continue;
        }

        match review_code(config, prompts, issue_text, &lead_res.architectural_plan).await {
            Some(rev) if rev.is_approved => {
                info!("Review approved on attempt {}", attempt);
                return DevLoopResult {
                    success: true,
                    branch_name: branch_name.to_string(),
                    thought_process: dev_res.thought_process,
                };
            }
            Some(rev) => {
                let fb = rev.feedback_thread.unwrap_or_default();
                feedback_history = truncate_to(&fb, 2000);
                warn!(feedback = %feedback_history, attempt, "Review rejected");
            }
            None => {
                warn!(
                    attempt,
                    "Reviewer returned unparseable JSON — treating as soft rejection"
                );
                feedback_history =
                    "The reviewer could not parse its output. \
                     This likely means the code diff looked incomplete. \
                     Ensure your implementation is 100% complete with no placeholders or cut-off content."
                        .to_string();
            }
        }
    }

    warn!("Max attempts ({}) reached without approval", max_attempts);
    DevLoopResult {
        success: false,
        branch_name: String::new(),
        thought_process: String::new(),
    }
}

async fn run_dev_tool_loop(
    config: &Config,
    dev_prompt: &str,
    conversation: &mut Vec<serde_json::Value>,
    use_anthropic_cache: bool,
    attempt: usize,
) -> Result<String, String> {
    let mut read_turns = 0usize;

    loop {
        let turn = llm_client::ask_dev_turn(
            config,
            dev_prompt,
            conversation,
            use_anthropic_cache,
        )
        .await;

        match turn {
            DevTurnResult::ApplyPatch(payload) => {
                info!(attempt, "Agent called apply_patch");
                return Ok(payload);
            }

            DevTurnResult::ReadFile { tool_use_id, file_path, start_line, end_line } => {
                read_turns += 1;
                if read_turns > MAX_READ_FILE_TURNS {
                    return Err(format!(
                        "Agent called read_file {} times without calling apply_patch. \
                         You must call apply_patch to submit your changes.",
                        read_turns
                    ));
                }

                info!(
                    attempt,
                    read_turns,
                    file = %file_path,
                    start = ?start_line,
                    end = ?end_line,
                    "Agent is reading a file (Read-Before-Write)"
                );

                let file_content = serve_read_file(config, &file_path, start_line, end_line);

                append_read_file_turn(
                    conversation,
                    &tool_use_id,
                    &file_path,
                    start_line,
                    end_line,
                    &file_content,
                    use_anthropic_cache,
                );
            }

            DevTurnResult::Error(e) => {
                return Err(e);
            }
        }
    }
}

fn serve_read_file(
    config: &Config,
    file_path: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> String {
    let full_path = config.workspace_dir.join(file_path);

    let content = match std::fs::read_to_string(&full_path) {
        Ok(c) => c,
        Err(e) => return format!("Error: could not read '{}': {}", file_path, e),
    };

    if start_line.is_none() && end_line.is_none() {
        let numbered: String = content
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{:>4}: {}\n", i + 1, l))
            .collect();
        return format!(
            "File: {} ({} lines)\n\n{}",
            file_path,
            content.lines().count(),
            numbered
        );
    }

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = start_line.map(|s| s.saturating_sub(1)).unwrap_or(0).min(total);
    let end = end_line.map(|e| e.min(total)).unwrap_or(total);

    if start >= end {
        return format!(
            "Error: invalid line range {}—{} for '{}' ({} lines total)",
            start + 1, end, file_path, total
        );
    }

    let numbered: String = lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{:>4}: {}\n", start + i + 1, l))
        .collect();

    format!(
        "File: {} (lines {}—{} of {})\n\n{}",
        file_path,
        start + 1,
        end,
        total,
        numbered
    )
}

fn append_read_file_turn(
    conversation: &mut Vec<serde_json::Value>,
    tool_use_id: &str,
    file_path: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
    file_content: &str,
    use_anthropic_format: bool,
) {
    let input = {
        let mut m = serde_json::Map::new();
        m.insert("file_path".to_string(), serde_json::json!(file_path));
        if let Some(s) = start_line {
            m.insert("start_line".to_string(), serde_json::json!(s));
        }
        if let Some(e) = end_line {
            m.insert("end_line".to_string(), serde_json::json!(e));
        }
        serde_json::Value::Object(m)
    };

    if use_anthropic_format {
        conversation.push(serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": tool_use_id,
                "name": "read_file",
                "input": input
            }]
        }));
        conversation.push(serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": file_content
            }]
        }));
    } else {
        let args = serde_json::to_string(&input).unwrap_or_default();
        conversation.push(serde_json::json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": tool_use_id,
                "type": "function",
                "function": {
                    "name": "read_file",
                    "arguments": args
                }
            }]
        }));
        conversation.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": tool_use_id,
            "content": file_content
        }));
    }
}

fn build_dynamic_dev_prompt(
    issue_text: &str,
    lead_res: &TeamLeaderResponse,
    feedback_history: &str,
    attempt: usize,
    max_attempts: usize,
) -> String {
    let attempt_warning = if attempt > 1 {
        format!(
            "\n⚠️  ATTEMPT {}/{}: Previous attempt(s) failed. Study the REVIEWER FEEDBACK carefully.\n",
            attempt, max_attempts
        )
    } else {
        String::new()
    };

    format!(
        "{attempt_warning}\
         Issue: {issue}\n\
         Architectural Plan: {plan}\n\
         \n\
         WORKFLOW:\n\
         1. OPTIONAL: Call `read_file` one or more times to inspect any file before writing patches.\n\
            - Use `read_file` when you need to verify exact indentation, surrounding context, or function signatures.\n\
            - You may call it multiple times on different files or line ranges.\n\
         2. REQUIRED: Call `apply_patch` once with ALL your file modifications.\n\
         \n\
         CRITICAL OUTPUT RULES (for apply_patch):\n\
         1. Output ONLY a raw JSON object — absolutely zero prose before or after.\n\
         2. The JSON MUST be 100%% complete and syntactically valid — never stop mid-object.\n\
         3. Inside JSON string values: \\n for newlines, \\\" for quotes, \\\\ for backslash.\n\
         4. new_content / replace_block MUST contain the ENTIRE implementation — no placeholders, no '// ...'.\n\
         5. Never truncate. If a function is long, write every single line.\n\
         \n\
         REVIEWER FEEDBACK TO FIX: {feedback}",
        attempt_warning = attempt_warning,
        issue = issue_text,
        plan = lead_res.architectural_plan,
        feedback = feedback_history,
    )
}

async fn review_code(
    config: &Config,
    prompts: &Prompts,
    issue_text: &str,
    architectural_plan: &str,
) -> Option<ReviewerResponse> {
    info!("Reviewer analysing code...");

    let diff = git_local::get_diff_against_base(config).unwrap_or_default();

    if diff.trim().is_empty() {
        warn!("Diff is empty — no changes were committed, auto-rejecting");
        return Some(ReviewerResponse {
            thought_process: "Diff was empty — no code was changed.".to_string(),
            is_approved: false,
            feedback_thread: Some(
                "CRITICAL: The diff was completely empty. \
                 No files were modified. You must actually write code changes."
                    .to_string(),
            ),
        });
    }

    let input = format!(
        "Original Issue: {}\nArchitectural Plan (Scope): {}\nDiff:\n{}",
        issue_text,
        architectural_plan,
        truncate_to(&diff, 12_000)
    );

    let raw = match llm_client::ask(config, &prompts.reviewer, &input).await {
        Ok(r) => r,
        Err(e) => {
            error!(err = %e, "LLM request failed during review");
            return None;
        }
    };

    match serde_json::from_str::<ReviewerResponse>(&raw) {
        Ok(r) => Some(r),
        Err(e) => {
            error!(err = %e, raw = %raw, "Reviewer returned invalid JSON");
            None
        }
    }
}

async fn deliver_pr(
    config: &Config,
    issue_id: &str,
    branch_name: &str,
    thought_process: &str,
    repo_service: &dyn RepoService,
) -> Result<String, Box<dyn std::error::Error>> {
    git_local::push_to_remote(config, branch_name)?;

    let title = format!("fix: Resolve Issue #{} — AI Auto PR", issue_id);
    let body = format!(
        "## Automated PR by BAGENTS\n\n\
         **Closes:** #{}\n\n\
         **Agent thought process:**\n{}\n\n\
         ---\n\
         *Generated by BAGENTS — AI Software Team.*",
        issue_id, thought_process
    );

    repo_service.create_pull_request(&title, &body, branch_name, &config.base_branch).await
}

async fn process_issue(
    config: &Config,
    prompts: &Prompts,
    issue: &Issue,
    tracker: &dyn IssueTracker,
    repo_service: &dyn RepoService,
) -> bool {
    let branch_name = format!("feature/issue-{}", issue.id);

    if let Err(e) = git_local::setup_branch(config, &branch_name) {
        error!(err = %e, "Could not setup branch for issue");
        return false;
    }

    let issue_body = issue.body.clone();

    let comments = tracker.fetch_issue_comments(&issue.id)
        .await
        .unwrap_or_default();

    let issue_text = format!(
        "Title: {}\nBody: {}\n\n--- COMMENTS HISTORY ---\n{}",
        issue.title, issue_body, comments
    );

    let repo_map = file_system::get_repo_map(config);
    let repo_tree = file_system::get_repo_tree(config);

    let lead_res = match plan_issue(config, prompts, &issue_text, &repo_map).await {
        Some(r) => r,
        None => {
            error!(issue = %issue.id, "Planning failed — skipping issue");
            return false;
        }
    };

    info!(
        agent = %lead_res.assigned_agent,
        plan = %lead_res.architectural_plan,
        files = ?lead_res.files_to_read,
        "Plan ready"
    );

    let TokenBudgetResult {
        lead_res,
        remaining_files,
        was_truncated,
    } = apply_token_budget(lead_res);

    let result = execute_dev_loop(
        config,
        prompts,
        &issue_text,
        &repo_tree,
        &lead_res,
        &branch_name,
    )
    .await;

    if !result.success {
        error!(issue = %issue.id, "Dev loop exhausted — no PR opened");
        let _ = git_local::reset_to_base(config);
        return false;
    }

    if was_truncated {
        let _ = git_local::push_to_remote(config, &branch_name);

        let comment = format!(
            "**[AUTO-CONTINUE] Partial completion**\n\n\
             ✅ Successfully updated: `{:?}`\n\n\
             ⏳ Still to process (next cycle): `{:?}`\n\n\
             *Picking this up automatically in the next cycle.*",
            lead_res.files_to_read, remaining_files
        );
        let _ = tracker.create_issue_comment(&issue.id, &comment).await;
        info!(
            issue = %issue.id,
            "AUTO-CONTINUE comment posted, branch saved"
        );

        return false;
    }

    match deliver_pr(
        config,
        &issue.id,
        &result.branch_name,
        &result.thought_process,
        repo_service,
    )
    .await
    {
        Ok(url) => {
            info!(url = %url, "Pull request opened");
        }
        Err(e) => {
            error!(err = %e, "Failed to open pull request");
        }
    }

    let _ = git_local::reset_to_base(config);
    git_local::delete_local_branch(config, &result.branch_name);

    true
}

fn find_placeholder(dev_res: &DeveloperResponse) -> Option<(String, String)> {
    for file in &dev_res.files_to_modify {
        let sources = [
            file.replace_block.as_str(),
            file.new_content.as_str(),
            file.target_chunk.as_str(),
        ];
        for source in &sources {
            for marker in PLACEHOLDER_MARKERS {
                if source.contains(marker) {
                    return Some((file.file_path.clone(), marker.to_string()));
                }
            }
        }
    }
    None
}

fn truncate_to<S: AsRef<str>>(s: S, max_chars: usize) -> String {
    let s = s.as_ref();
    if let Some((byte_idx, _)) = s.char_indices().nth(max_chars) {
        format!("{}…[TRUNCATED]", &s[..byte_idx])
    } else {
        s.to_string()
    }
}
