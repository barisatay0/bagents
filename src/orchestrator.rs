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
];

/// Maximum number of files passed to a developer agent per cycle.
/// Keeping this at 1 protects the context window. Increase as models improve.
const MAX_FILES_PER_CYCLE: usize = 1;

// ── entry point ──────────────────────────────────────────────────────────────

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
/// Returns `None` if the LLM returns unparseable JSON after logging the error.
async fn plan_issue(
    config: &Config,
    prompts: &Prompts,
    issue_text: &str,
    repo_tree: &str,
) -> Option<TeamLeaderResponse> {
    info!("Team leader is planning...");

    let input = format!("{}  Issue Context: {}", repo_tree, issue_text);

    let raw = match llm_client::ask(config, &prompts.team_lead, &input).await {
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
/// On each attempt the workspace is reset to main first to ensure the developer
/// always works from a clean baseline rather than layering on previous attempts.
async fn execute_dev_loop(
    config: &Config,
    prompts: &Prompts,
    issue_text: &str,
    repo_tree: &str,
    lead_res: &TeamLeaderResponse,
    branch_name: &str,
) -> DevLoopResult {
    let max_attempts = 5;
    let mut feedback_history = String::new();

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

    for attempt in 1..=max_attempts {
        info!(attempt, max_attempts, agent = %lead_res.assigned_agent, "Developer writing code");

        // Reset uncommitted changes instead of hard resetting to main.
        // This ensures we stay on the current issue branch but wipe bad LLM code from previous attempts.
        if let Err(e) = git_local::reset_working_tree(config) {
            error!(err = %e, "Could not reset working tree before attempt");
            break;
        }

        let dev_input = format!(
            "Project Context: '{}/{}'.\n{}\n{}\n{}\nIssue: {}\nArchitectural Plan: {}\nREVIEWER FEEDBACK TO FIX: {}",
            config.github_owner,
            config.github_repo,
            repo_tree,
            semantic_outlines,
            specific_files,
            issue_text,
            lead_res.architectural_plan,
            feedback_history
        );

        let dev_prompt = prompts.for_agent(&lead_res.assigned_agent);

        let raw = match llm_client::ask(config, dev_prompt, &dev_input).await {
            Ok(r) => r,
            Err(e) => {
                error!(err = %e, "LLM request failed during dev loop");
                feedback_history = format!("SYSTEM ERROR: LLM request failed: {}", e);
                continue;
            }
        };

        let dev_res: DeveloperResponse = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(e) => {
                warn!(err = %e, "Developer returned invalid JSON — retrying");
                let snippet = raw.chars().take(300).collect::<String>();
                feedback_history = format!(
                    "CRITICAL: Your last response was NOT valid JSON. serde error: '{}'. \
                     The response started with: {:?}. \
                     Output ONLY a raw JSON object. No thinking text, no markdown, no prose before or after the {{}}.",
                    e, snippet
                );
                continue;
            }
        };

        info!(thought = %dev_res.thought_process, "Developer response parsed");

        // Placeholder check — report which file triggered it
        if let Some(bad_file) = find_placeholder(&dev_res) {
            warn!(file = %bad_file, "Placeholder detected — rejecting");
            feedback_history = format!(
                "CRITICAL ERROR: File '{}' contains placeholder code (TODO/FIXME/unimplemented!/etc.). \
                 Write the COMPLETE, production-ready implementation. Do not skip any logic.",
                bad_file
            );
            continue;
        }

        // Apply changes
        if let Err(e) = file_system::apply_modifications(config, dev_res.files_to_modify.clone()) {
            warn!(err = %e, "Patch failed");
            feedback_history = format!(
                "CRITICAL ERROR: Failed to apply patch. The target_chunk or search_block did NOT match. \
                 Error: {}",
                e
            );
            continue;
        }

        if let Err(verify_err) = git_local::run_verification(config) {
            let is_relevant_error = dev_res.files_to_modify.iter().any(|m| {
                let file_name = std::path::Path::new(&m.file_path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                verify_err.contains(&m.file_path) || verify_err.contains(file_name.as_ref())
            });

            if is_relevant_error {
                warn!(
                    attempt,
                    "Verification failed on modified files, sending back to dev"
                );
                feedback_history = format!(
                    "CRITICAL VERIFICATION ERROR: Your code failed the build/linter. \
                                     Fix ONLY the errors in the files you modified. Error output:\n{}",
                    truncate_to(verify_err, 1500)
                );
                continue;
            } else {
                warn!(
                    attempt,
                    "Verification failed, but errors are in OUT-OF-SCOPE files. Bypassing check."
                );
            }
        }
        // ───────────────────────────────────────────────────────────────────

        let commit_msg = format!("feat: Resolve issue (attempt {})", attempt);
        let _ = git_local::commit_changes(config, &commit_msg);

        // Review
        match review_code(config, prompts, issue_text, &lead_res.architectural_plan).await {
            Some(rev) if rev.is_approved => {
                info!("Review approved");
                return DevLoopResult {
                    success: true,
                    branch_name: branch_name.to_string(),
                    thought_process: dev_res.thought_process,
                };
            }
            Some(rev) => {
                feedback_history = truncate_to(rev.feedback_thread.unwrap_or_default(), 1500);
                warn!(feedback = %feedback_history, attempt, "Review rejected");
            }
            None => {
                warn!(
                    attempt,
                    "Reviewer returned unparseable JSON — treating as rejection"
                );
                feedback_history =
                    "The reviewer could not parse its own output. Please ensure your code is complete and correct.".to_string();
            }
        }
    }

    warn!("Max attempts reached without approval");
    DevLoopResult {
        success: false,
        branch_name: String::new(),
        thought_process: String::new(),
    }
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
    let input = format!(
        "Original Issue: {}\nArchitectural Plan (Scope): {}\nDiff:\n{}",
        issue_text, architectural_plan, diff
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

    let title = format!("Resolve Issue #{} — Auto AI PR", issue_number);
    let body = format!(
        "**Automated PR by AI Software Factory**\n\n**Agent thought process:** {}\n\nCloses #{}",
        thought_process, issue_number
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

    let repo_tree = file_system::get_repo_tree(config);

    // Plan
    let lead_res = match plan_issue(config, prompts, &issue_text, &repo_tree).await {
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

    // Token budget
    let TokenBudgetResult {
        lead_res,
        remaining_files,
        was_truncated,
    } = apply_token_budget(lead_res);

    // Dev loop
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
             Successfully updated: `{:?}`.\n\n\
             Still to process (next cycle): `{:?}`.\n\n\
             *Picking this up automatically in the next cycle.*",
            lead_res.files_to_read, remaining_files
        );
        let _ = github::create_issue_comment(config, issue.number, &comment).await;
        info!(
            issue = issue.number,
            "AUTO-CONTINUE comment posted, branch saved"
        );

        // Do not mark as fully processed — leave it for the next cycle
        // We also don't open a PR yet, and we don't delete the local branch.
        return false;
    }

    // Deliver PR (Only executes if was_truncated is false, meaning the entire job is done)
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

/// Return the file path of the first file containing a known placeholder marker,
/// or `None` if the submission looks complete.
fn find_placeholder(dev_res: &DeveloperResponse) -> Option<String> {
    for file in &dev_res.files_to_modify {
        for marker in PLACEHOLDER_MARKERS {
            if file.replace_block.contains(marker) || file.new_content.contains(marker) {
                return Some(file.file_path.clone());
            }
        }
    }
    None
}
/// Truncate a string to `max_chars` characters.
/// Appends `…[TRUNCATED]` when cut so the model knows output was clipped.
/// Keeps `feedback_history` token-bounded across retry attempts.
fn truncate_to<S: AsRef<str>>(s: S, max_chars: usize) -> String {
    let s = s.as_ref();
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars).collect();
    format!("{cut}…[TRUNCATED]")
}
