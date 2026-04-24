use std::collections::HashSet;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

use crate::clients::llm_client;
use crate::config::Config;
use crate::models::{
    developer_response::DeveloperResponse, reviewer_response::ReviewerResponse,
    teamlead_response::TeamLeaderResponse,
};
use crate::prompts::Prompts;
use crate::services::{file_system, git_local, github};

/// Placeholder markers that indicate an agent submitted incomplete code.
const PLACEHOLDER_MARKERS: &[&str] = &[
    "// TODO",
    "// FIXME",
    "// HACK",
    "// XXX",
    "todo!()",
    "unimplemented!()",
    "... existing code ...",
    "/* TODO */",
    "# TODO",
    "FIXME",
    "// ...",
    "{/* ... */}",
    "// rest of",
    "// ... rest",
    "/* rest of",
];

/// Maximum number of files passed to a developer agent per cycle.
/// Keeping this at 2 gives better context while still protecting the window.
const MAX_FILES_PER_CYCLE: usize = 2;

// ── entry point ───────────────────────────────────────────────────────────────

/// Main factory loop. Polls GitHub for open issues, orchestrates agents,
/// reviews code, and opens pull requests until interrupted.
pub async fn run_factory(
    config: &Config,
    prompts: &Prompts,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Factory started — polling for issues continuously");

    let mut processed_issues: HashSet<u64> = HashSet::new();

    loop {
        info!("Checking GitHub for open issues...");

        let issues = match github::fetch_open_issues(config).await {
            Ok(i) => i,
            Err(e) => {
                error!(err = %e, "GitHub API error — retrying in 60s");
                sleep(Duration::from_secs(60)).await;
                continue;
            }
        };

        let target_issue = match issues
            .into_iter()
            .find(|i| !processed_issues.contains(&i.number))
        {
            Some(i) => i,
            None => {
                info!("No new issues — resting for 30s");
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        info!(
            issue = target_issue.number,
            title = %target_issue.title,
            "Processing issue"
        );

        // Always start from a clean main branch
        if let Err(e) = git_local::reset_to_main(config) {
            error!(err = %e, "Could not reset to main — skipping issue");
            sleep(Duration::from_secs(10)).await;
            continue;
        }

        let is_successful = process_issue(config, prompts, &target_issue).await;

        if is_successful {
            info!(issue = target_issue.number, "Issue completed successfully");
            processed_issues.insert(target_issue.number);
        } else {
            warn!(
                issue = target_issue.number,
                "Issue failed, exhausted, or deferred"
            );
        }

        sleep(Duration::from_secs(10)).await;
    }
}

// ── stage 1: plan ─────────────────────────────────────────────────────────────

/// Ask the team lead to analyse the issue and produce an architectural plan.
///
/// The team lead receives:
///   • The repository map (file paths + symbol signatures, no bodies) as a
///     **cached** static context block — this stays constant for all issues in
///     the same run, so Anthropic will serve it from cache on the 2nd+ call.
///   • The issue text as the dynamic (uncached) user prompt.
///
/// Returns `None` if the LLM returns unparseable JSON after logging the error.
async fn plan_issue(
    config: &Config,
    prompts: &Prompts,
    issue_text: &str,
    repo_map: &str,
) -> Option<TeamLeaderResponse> {
    info!("Team leader is planning...");

    // The repo map is static across issues within a run → ideal cache candidate.
    // The issue text changes every call → must stay dynamic (uncached).
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
        repo_map,   // static context — cached on Anthropic
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

// ── stage 2: token budget ─────────────────────────────────────────────────────

struct TokenBudgetResult {
    lead_res: TeamLeaderResponse,
    remaining_files: Vec<String>,
    was_truncated: bool,
}

/// Enforce the per-cycle file limit. Injects a SYSTEM OVERRIDE into the
/// architectural plan and records the spilled files for the AUTO-CONTINUE comment.
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

// ── stage 3: dev loop ─────────────────────────────────────────────────────────

struct DevLoopResult {
    success: bool,
    branch_name: String,
    thought_process: String,
}

/// Run the developer → commit → review feedback loop for up to `max_attempts`.
///
/// On each attempt the workspace is reset to the branch HEAD so the developer
/// always works from a clean baseline rather than layering on previous attempts.
///
/// The static context (repo tree + file contents) is passed via
/// `ask_large_with_context` so that Anthropic's prompt cache covers the
/// expensive file-reading portion.  Only the dynamic per-attempt feedback
/// changes between retries, keeping cached token reuse high.
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
    // Track consecutive JSON failures so we can vary the repair strategy
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

    // ── Build the static context block ────────────────────────────────────────
    // This portion is identical across all retry attempts for this issue, so it
    // qualifies for prompt caching.  We assemble it once here and pass it as
    // `static_context` to `ask_large_with_context` on every attempt.
    let static_context = format!(
        "Project: '{owner}/{repo}'.\n\
         \n\
         {repo_tree}\n\
         \n\
         {semantic_outlines}\n\
         \n\
         {specific_files}",
        owner = config.github_owner,
        repo = config.github_repo,
        repo_tree = repo_tree,
        semantic_outlines = semantic_outlines,
        specific_files = specific_files,
    );

    let dev_prompt = prompts.for_agent(&lead_res.assigned_agent);

    for attempt in 1..=max_attempts {
        info!(attempt, max_attempts, agent = %lead_res.assigned_agent, "Developer writing code");

        // Reset uncommitted changes — stay on the issue branch but wipe bad LLM code.
        if let Err(e) = git_local::reset_working_tree(config) {
            error!(err = %e, "Could not reset working tree before attempt");
            break;
        }

        // The dynamic part changes on every attempt (attempt number + feedback).
        let dynamic_prompt = build_dynamic_dev_prompt(
            issue_text,
            lead_res,
            &feedback_history,
            attempt,
            max_attempts,
        );

        // Use the large-token variant with the static context cached separately.
        let raw = match llm_client::ask_large_with_context(
            config,
            dev_prompt,
            &static_context,
            &dynamic_prompt,
        )
        .await
        {
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

        // ── JSON parse ────────────────────────────────────────────────────────
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
                    // Escalate: give them a concrete skeleton to fill
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

        // ── Output completeness checks ─────────────────────────────────────────

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

        // Placeholder check — report which file triggered it
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

        // ── Apply changes ──────────────────────────────────────────────────────
        if let Err(e) = file_system::apply_modifications(config, dev_res.files_to_modify.clone()) {
            warn!(err = %e, "Patch failed");
            feedback_history = format!(
                "CRITICAL ERROR: Failed to apply your patch. Details:\n{}\n\n\
                 Instructions:\n\
                 - If using `target_chunk`: the name must EXACTLY match one from the 'Available chunks' list shown in SEMANTIC FILE OUTLINES.\n\
                 - If using `search_block`: copy the block CHARACTER-FOR-CHARACTER from the file content. Include 3+ lines of context.\n\
                 - Do NOT invent chunk names or approximate search blocks.",
                e
            );
            continue;
        }

        // ── Verify ─────────────────────────────────────────────────────────────
        if let Err(verify_err) = git_local::run_verification(config) {
            let is_relevant_error = dev_res.files_to_modify.iter().any(|m| {
                let file_name = std::path::Path::new(&m.file_path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                verify_err.contains(&m.file_path) || verify_err.contains(file_name.as_ref())
            });

            if is_relevant_error {
                warn!(attempt, "Verification failed on modified files");
                feedback_history = format!(
                    "CRITICAL VERIFICATION ERROR: Your code failed the build/linter.\n\
                     Fix ONLY the errors in the files you modified. Do not change anything else.\n\
                     Error output:\n{}",
                    truncate_to(verify_err, 2000)
                );
                continue;
            } else {
                warn!(
                    attempt,
                    "Verification failed but errors are in out-of-scope files — bypassing"
                );
            }
        }

        // ── Commit ─────────────────────────────────────────────────────────────
        let commit_msg = format!(
            "feat: Resolve issue #{} (attempt {})\n\n{}",
            branch_name.trim_start_matches("feature/issue-"),
            attempt,
            truncate_to(&dev_res.thought_process, 500)
        );
        let _ = git_local::commit_changes(config, &commit_msg);

        // ── Review ─────────────────────────────────────────────────────────────
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

/// Build the dynamic portion of the developer prompt that changes between attempts.
///
/// The static portion (repo tree, file contents) is passed separately as
/// `static_context` so it can be prompt-cached and reused across retries.
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
         CRITICAL OUTPUT RULES:\n\
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

// ── stage 4: review ───────────────────────────────────────────────────────────

/// Ask the reviewer to evaluate the current diff. Returns `None` on parse error.
async fn review_code(
    config: &Config,
    prompts: &Prompts,
    issue_text: &str,
    architectural_plan: &str,
) -> Option<ReviewerResponse> {
    info!("Reviewer analysing code...");

    let diff = git_local::get_diff_against_main(config).unwrap_or_default();

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

    // Static context for the reviewer: plan + diff.  The diff changes every
    // review call so there is no meaningful cache hit here; we use plain `ask`.
    let input = format!(
        "Original Issue: {}\nArchitectural Plan (Scope): {}\nDiff:\n{}",
        issue_text,
        architectural_plan,
        truncate_to(diff, 12_000)
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

// ── stage 5: deliver ──────────────────────────────────────────────────────────

/// Push the approved branch and open a pull request. Returns the PR URL.
async fn deliver_pr(
    config: &Config,
    issue_number: u64,
    branch_name: &str,
    thought_process: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    git_local::push_to_remote(config, branch_name)?;

    let title = format!("fix: Resolve Issue #{} — AI Auto PR", issue_number);
    let body = format!(
        "## Automated PR by BAGENTS\n\n\
         **Closes:** #{}\n\n\
         **Agent thought process:**\n{}\n\n\
         ---\n\
         *Generated by BAGENTS — the autonomous AI software factory.*",
        issue_number, thought_process
    );

    github::create_pull_request(config, &title, &body, branch_name, "main").await
}

// ── top-level per-issue workflow ──────────────────────────────────────────────

/// Full workflow for a single issue. Returns `true` if a PR was opened.
async fn process_issue(
    config: &Config,
    prompts: &Prompts,
    issue: &octocrab::models::issues::Issue,
) -> bool {
    let branch_name = format!("feature/issue-{}", issue.number);

    if let Err(e) = git_local::setup_branch(config, &branch_name) {
        error!(err = %e, "Could not setup branch for issue");
        return false;
    }

    let issue_body = issue.body.clone().unwrap_or_default();

    let comments = github::fetch_issue_comments(config, issue.number)
        .await
        .unwrap_or_default();

    let issue_text = format!(
        "Title: {}\nBody: {}\n\n--- COMMENTS HISTORY ---\n{}",
        issue.title, issue_body, comments
    );

    // ── Build context strings ─────────────────────────────────────────────────
    //
    // repo_map  → compact symbol-signature map used by the Team Lead for planning.
    //             Constant across all issues → excellent cache candidate.
    //
    // repo_tree → flat file-path listing used inside the developer prompt as a
    //             quick reference.  Also static, but smaller than the map.
    //
    let repo_map = file_system::get_repo_map(config);
    let repo_tree = file_system::get_repo_tree(config);

    // ── Plan ──────────────────────────────────────────────────────────────────
    let lead_res = match plan_issue(config, prompts, &issue_text, &repo_map).await {
        Some(r) => r,
        None => {
            error!(issue = issue.number, "Planning failed — skipping issue");
            return false;
        }
    };

    info!(
        agent = %lead_res.assigned_agent,
        plan = %lead_res.architectural_plan,
        files = ?lead_res.files_to_read,
        "Plan ready"
    );

    // ── Token budget ──────────────────────────────────────────────────────────
    let TokenBudgetResult {
        lead_res,
        remaining_files,
        was_truncated,
    } = apply_token_budget(lead_res);

    // ── Dev loop ──────────────────────────────────────────────────────────────
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
        error!(issue = issue.number, "Dev loop exhausted — no PR opened");
        let _ = git_local::reset_to_main(config);
        return false;
    }

    // AUTO-CONTINUE comment if we deferred files
    if was_truncated {
        // Push the partial work to the remote branch so it's not lost
        let _ = git_local::push_to_remote(config, &branch_name);

        let comment = format!(
            "**[AUTO-CONTINUE] Partial completion**\n\n\
             ✅ Successfully updated: `{:?}`\n\n\
             ⏳ Still to process (next cycle): `{:?}`\n\n\
             *Picking this up automatically in the next cycle.*",
            lead_res.files_to_read, remaining_files
        );
        let _ = github::create_issue_comment(config, issue.number, &comment).await;
        info!(
            issue = issue.number,
            "AUTO-CONTINUE comment posted, branch saved"
        );

        // Do not mark as fully processed — leave it for the next cycle
        return false;
    }

    // ── Deliver PR ────────────────────────────────────────────────────────────
    match deliver_pr(
        config,
        issue.number,
        &result.branch_name,
        &result.thought_process,
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

    // Cleanup local branch
    git_local::delete_local_branch(config, &result.branch_name);
    let _ = git_local::reset_to_main(config);

    true
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Return the file path and triggering marker of the first file containing a
/// known placeholder marker, or `None` if the submission looks complete.
fn find_placeholder(dev_res: &DeveloperResponse) -> Option<(String, String)> {
    for file in &dev_res.files_to_modify {
        let sources = [
            file.replace_block.as_str(),
            file.new_content.as_str(),
            file.target_chunk.as_str(), // paranoia
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

/// Truncate a string to `max_chars` characters.
/// Appends `…[TRUNCATED]` when cut so the model knows output was clipped.
fn truncate_to<S: AsRef<str>>(s: S, max_chars: usize) -> String {
    let s = s.as_ref();
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars).collect();
    format!("{cut}…[TRUNCATED]")
}
