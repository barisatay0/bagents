use crate::clients::llm_client;
use crate::models::{
    developer_response::DeveloperResponse, reviewer_response::ReviewerResponse,
    teamlead_response::TeamLeaderResponse,
};
use crate::services::{file_system, git_local, github};
use std::fs;

pub async fn run_factory() -> Result<(), Box<dyn std::error::Error>> {
    println!("Checking GitHub for open issues...");
    let issues = github::fetch_open_issues().await?;

    if issues.is_empty() {
        println!("No open issues found. Factory is resting.");
        return Ok(());
    }

    let target_issue = &issues[0]; // Process the first open issue
    let issue_body = target_issue.body.clone().unwrap_or_default();
    let issue_text = format!("Title: {} Body: {}", target_issue.title, issue_body);

    println!(
        "Processing Issue #{}: {}",
        target_issue.number, target_issue.title
    );

    // 1. TEAM LEADER PHASE
    let team_lead_prompt = fs::read_to_string("config/team_lead.md")?;
    println!("Team Leader is thinking...");

    let lead_raw = llm_client::ask(&team_lead_prompt, &issue_text).await?;

    println!("--- LLM Response ---  {} ----------------------", lead_raw);

    let lead_res: TeamLeaderResponse =
        serde_json::from_str(&lead_raw).expect("Failed to parse Team Leader JSON");

    println!("Agent Assigned: {}", lead_res.assigned_agent);
    println!("Architecture Plan: {}", lead_res.architectural_plan);

    let dev_prompt_path = format!("config/{}.md", lead_res.assigned_agent);
    let dev_prompt = fs::read_to_string(&dev_prompt_path).unwrap_or_else(|_| {
        println!("Agent config not found, falling back to backend_dev");
        fs::read_to_string("config/backend_dev.md").unwrap()
    });

    let repo_owner = std::env::var("GITHUB_OWNER").unwrap_or_default();
    let repo_name = std::env::var("GITHUB_REPO").unwrap_or_default();

    let mut feedback_history = String::new();
    let mut attempt = 1;
    let max_attempts = 3;

    loop {
        println!("[ATTEMPT {}/{}]", attempt, max_attempts);
        println!("{} is writing/fixing code...", lead_res.assigned_agent);

        let dev_input = if feedback_history.is_empty() {
            format!(
                "Project Context: You are working on the repository '{}/{}'.  Issue: {}  Architectural Plan: {}",
                repo_owner, repo_name, issue_text, lead_res.architectural_plan
            )
        } else {
            format!(
                "Project Context: You are working on the repository '{}/{}'.  Issue: {}  Architectural Plan: {}  REVIEWER FEEDBACK TO FIX: {}",
                repo_owner, repo_name, issue_text, lead_res.architectural_plan, feedback_history
            )
        };

        let dev_raw = llm_client::ask(&dev_prompt, &dev_input).await?;
        let dev_res: DeveloperResponse =
            serde_json::from_str(&dev_raw).expect("Failed to parse Developer JSON");

        println!("Dev Thought: {}", dev_res.thought_process);

        git_local::create_branch(&dev_res.branch_name)?;
        file_system::apply_modifications(dev_res.files_to_modify)?;

        let commit_msg = format!(
            "feat: Resolve issue #{} (Attempt {})",
            target_issue.number, attempt
        );
        let _ = git_local::commit_changes(&commit_msg);

        // 4. REVIEWER PHASE
        println!("Reviewer is analyzing the code...");
        let reviewer_prompt = fs::read_to_string("config/reviewer.md")?;
        let git_diff = git_local::get_diff_against_main().unwrap_or_default();
        let reviewer_input = format!("Here is the git diff for the new feature: {}", git_diff);

        let rev_raw = llm_client::ask(&reviewer_prompt, &reviewer_input).await?;
        let rev_res: ReviewerResponse =
            serde_json::from_str(&rev_raw).expect("Failed to parse Reviewer JSON");

        if rev_res.is_approved {
            println!("REVIEW APPROVED: Code is clean and production-ready!");

            git_local::push_to_remote(&dev_res.branch_name)?;

            println!("Opening Pull Request on GitHub...");
            let pr_title = format!("Resolve Issue #{} - Auto AI PR", target_issue.number);
            let pr_body = format!(
                "**Automated PR by AI Software Factory**  **Agent Thought Process:** {}  **Reviewer Notes:** Approved after {} attempts.",
                dev_res.thought_process, attempt
            );

            match github::create_pull_request(&pr_title, &pr_body, &dev_res.branch_name, "main")
                .await
            {
                Ok(url) => println!("🎉 BOOM! Pull Request opened successfully: {}", url),
                Err(e) => println!("Failed to open PR: {}", e),
            }
            break;
        } else {
            println!("❌ REVIEW REJECTED: Changes required.");
            if let Some(feedback) = rev_res.feedback_thread {
                println!("Feedback: {}", feedback);
                feedback_history = feedback.clone();

                println!("Posting Reviewer feedback to GitHub Issue...");
                let comment_body = format!(
                    "**Reviewer Feedback (Attempt {}):**\n{}",
                    attempt, feedback_history
                );
                let _ = github::create_issue_comment(target_issue.number, &comment_body).await;
            }

            attempt += 1;
            if attempt > max_attempts {
                println!("MAX ATTEMPTS REACHED! Factory is halting to prevent infinite loops.");
                break;
            }
        }
    }

    println!("Workflow completed for Issue #{}!", target_issue.number);
    Ok(())
}
