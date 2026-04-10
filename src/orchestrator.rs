use crate::clients::llm_client;
use crate::models::{
    developer_response::DeveloperResponse,
    reviewer_response::ReviewerResponse,
    teamlead_response::TeamLeaderResponse,
};
use crate::services::{file_system, git_local, github};
use std::collections::HashSet;
use std::fs;
use tokio::time::{Duration, sleep};

pub async fn run_factory() -> Result<(), Box<dyn std::error::Error>> {
    info!("Factory started! Waiting for issues in continuous mode...");

    let mut processed_issues: HashSet<u64> = HashSet::new();

    loop {
        info!("Checking GitHub for open issues...");
        let issues = match github::fetch_open_issues().await {
            Ok(i) => i,
            Err(e) => {
                warn!("GitHub API Error: {}. Retrying later...", e);
                sleep(Duration::from_secs(60)).await;
                continue;
            }
        };

        let mut target_issue = None;
        for issue in issues {
            if !processed_issues.contains(&issue.number) {
                target_issue = Some(issue);
                break;
            }
        }

        let target_issue = match target_issue {
            Some(issue) => issue,
            None => {
                info!(" No new open issues found. Factory is resting for 30 seconds...");
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        let _ = git_local::reset_to_main();

        let issue_body = target_issue.body.clone().unwrap_or_default();
        let issue_text = format!("Title: {} Body: {}", target_issue.title, issue_body);

        info!("======================================= Processing Issue #{}: {} =======================================", target_issue.number, target_issue.title);

        info!("Reading repository context...");
        let repo_context = file_system::get_repo_context();

        let team_lead_prompt = fs::read_to_string("config/team_lead.md").unwrap();
        info!("Team Leader is thinking...");

        let team_lead_input = format!("{}  Issue: {}", repo_context, issue_text);
        let lead_raw = llm_client::ask(&team_lead_prompt, &team_lead_input).await?;
        let lead_res: TeamLeaderResponse =
            serde_json::from_str(&lead_raw).expect("Failed to parse Team Leader JSON");

        info!("Agent Assigned: {}", lead_res.assigned_agent);
        info!("Architecture Plan: {}", lead_res.architectural_plan);

        let dev_prompt_path = format!("config/{}.md", lead_res.assigned_agent);
        let dev_prompt = fs::read_to_string(&dev_prompt_path).unwrap_or_else(|_| {
            info!("Agent config not found, falling back to backend_dev");
            fs::read_to_string("config/backend_dev.md").unwrap()
        });

        let repo_owner = std::env::var("GITHUB_OWNER").unwrap_or_default();
        let repo_name = std::env::var("GITHUB_REPO").unwrap_or_default();

        let mut feedback_history = String::new();
        let mut attempt = 1;
        let max_attempts = 3;

        loop {
            info!(" [ATTEMPT {}/{}]", attempt, max_attempts);
            info!(" {} is writing/fixing code...", lead_res.assigned_agent);

            let dev_input = if feedback_history.is_empty() {
                format!(
                    "Project Context: You are working on the repository '{}/{}'.  {}  Issue: {} Architectural Plan: {}",
                    repo_owner,
                    repo_name,
                    repo_context,
                    issue_text,
                    lead_res.architectural_plan
                )
            } else {
                format!(
                    "Project Context: You are working on the repository '{}/{}'.  {}  Issue: {} Architectural Plan: {}   REVIEWER FEEDBACK TO FIX: {}",
                    repo_owner,
                    repo_name,
                    repo_context,
                    issue_text,
                    lead_res.architectural_plan,
                    feedback_history
                )
            };

            let dev_raw = llm_client::ask(&dev_prompt, &dev_input).await?;

            let dev_res: DeveloperResponse = match serde_json::from_str(&dev_raw) {
                Ok(res) => res,
                Err(e) => {
                    error!("LLM generated invalid JSON: {}. Forcing retry...", e);

                    feedback_history = format!(
                        "CRITICAL SYSTEM ERROR: Your last response was NOT valid JSON. The parser failed with: '{}'. You MUST strictly follow the JSON formatting rules, properly escape all double quotes (\\\") and newlines (\\n), and ensure the JSON is complete.",
                        e
                    );

                    attempt += 1;
                    if attempt > max_attempts {
                        info!("\\\u{1F6AB} MAX ATTEMPTS REACHED due to JSON parsing errors.");
                        break;
                    }
                    continue;
                }
            };

            info!("Dev Thought: {}", dev_res.thought_process);

            git_local::create_branch(&dev_res.branch_name)?;
            file_system::apply_modifications(dev_res.files_to_modify)?;

            let commit_msg = format!(
                "feat: Resolve issue #{} (Attempt {})",
                target_issue.number,
                attempt
            );
            let _ = git_local::commit_changes(&commit_msg);

            // 4. REVIEWER PHASE
            info!("Starting review phase: Code Reviewer is analyzing the git diff for branch '{}'", dev_res.branch_name);
            let reviewer_prompt = fs::read_to_string("config/reviewer.md").unwrap();
            let git_diff = git_local::get_diff_against_main().unwrap_or_default();
            let reviewer_input = format!(
                "Issue being solved: {}  Here is the git diff for the new feature: {}",
                issue_text,
                git_diff
            );

            let rev_raw = llm_client::ask(&reviewer_prompt, &reviewer_input).await?;
            let rev_res: ReviewerResponse =
                serde_json::from_str(&rev_raw).expect("Failed to parse Reviewer JSON");

            if rev_res.is_approved {
                info!("\\\u{1F44D} REVIEW APPROVED: Code is clean and production-ready!");
                git_local::push_to_remote(&dev_res.branch_name)?;

                info!("Opening Pull Request on GitHub...");
                let pr_title = format!("Resolve Issue #{} - Auto AI PR", target_issue.number);
                let pr_body = format!(
                    " **Automated PR by AI Software Factory**  **Agent Thought Process:** {}  **Reviewer Notes:** Approved after {} attempts.  Closes #{}",
                    dev_res.thought_process,
                    attempt,
                    target_issue.number
                );

                match github::create_pull_request(&pr_title, &pr_body, &dev_res.branch_name, "main")
                    .await
                {
                    Ok(url) => info!("\\\u{1F3E1} Pull Request opened successfully: {}", url),
                    Err(e) => error!("Failed to open PR: {}", e),
                }
                break;
            } else {
                warn!("\\\u{1F6AB} REVIEW REJECTED: Changes required.");
                if let Some(feedback) = rev_res.feedback_thread {
                    info!("Feedback: {}", feedback);
                    feedback_history = feedback.clone();

                    info!("Posting Reviewer feedback to GitHub Issue...");
                    let comment_body = format!(
                        "** Reviewer Feedback (Attempt {}):** {}",
                        attempt,
                        feedback_history
                    );
                    let _ = github::create_issue_comment(target_issue.number, &comment_body).await;
                }

                attempt += 1;
                if attempt > max_attempts {
                    info!("\\\u{1F6AB} MAX ATTEMPTS REACHED! Halting feedback loop.");
                    break;
                }
            }
        }

        processed_issues.insert(target_issue.number);
        info!("Workflow completed for Issue #{}. Adding to memory.", target_issue.number);

        sleep(Duration::from_secs(10)).await;
    }
}
