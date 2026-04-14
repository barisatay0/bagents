use crate::clients::llm_client;
use crate::models::{
    developer_response::DeveloperResponse, reviewer_response::ReviewerResponse,
    teamlead_response::TeamLeaderResponse,
};
use crate::services::{file_system, git_local, github};
use std::collections::HashSet;
use std::fs;
use tokio::time::{Duration, sleep};

pub async fn run_factory() -> Result<(), Box<dyn std::error::Error>> {
    println!("Factory started! Waiting for issues in continuous mode...");

    let mut processed_issues: HashSet<u64> = HashSet::new();

    loop {
        println!("Checking GitHub for open issues...");
        let issues = match github::fetch_open_issues().await {
            Ok(i) => i,
            Err(e) => {
                println!("GitHub API Error: {}. Retrying later...", e);
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
                println!(" No new open issues found. Factory is resting for 30 seconds...");
                sleep(Duration::from_secs(30)).await;
                continue;
            }
        };

        let _ = git_local::reset_to_main();

        let issue_body = target_issue.body.clone().unwrap_or_default();
        let issue_text = format!("Title: {} Body: {}", target_issue.title, issue_body);

        println!(
            " ======================================== Processing Issue #{}: {} ========================================",
            target_issue.number, target_issue.title
        );

        println!("Reading repository tree...");
        let repo_tree = file_system::get_repo_tree();

        let team_lead_prompt = fs::read_to_string("config/team_lead.md")?;
        println!("Team Leader is thinking...");

        let team_lead_input = format!("{}  Issue: {}", repo_tree, issue_text);
        let lead_raw = llm_client::ask(&team_lead_prompt, &team_lead_input).await?;
        let mut lead_res: TeamLeaderResponse =
            serde_json::from_str(&lead_raw).expect("Failed to parse Team Leader JSON");

        if lead_res.files_to_read.len() > 2 {
            println!(
                "WARNING: Team Leader requested {} files. Truncating to 2 to prevent LLM token exhaustion (EOF crash)!",
                lead_res.files_to_read.len()
            );
            lead_res.files_to_read.truncate(2);

            lead_res.architectural_plan = format!(
                "{} [SYSTEM OVERRIDE: Due to token limits, ONLY modify these specific files in this PR: {:?}. Ignore the rest for now.]",
                lead_res.architectural_plan, lead_res.files_to_read
            );
        }

        println!("Agent Assigned: {}", lead_res.assigned_agent);
        println!("Architecture Plan: {}", lead_res.architectural_plan);
        println!("Files to analyze: {:?}", lead_res.files_to_read);

        let specific_file_contents =
            file_system::read_specific_files(lead_res.files_to_read.clone());

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

        // YENİ: Görevin gerçekten bitip bitmediğini takip eden bayrak
        let is_successful = false;

        loop {
            println!(" [ATTEMPT {}/{}]", attempt, max_attempts);
            println!(" {} is writing/fixing code...", lead_res.assigned_agent);

            let dev_input = if feedback_history.is_empty() {
                format!(
                    "Project Context: '{}/{}'.\n{}\n{}\nIssue: {}\nArchitectural Plan: {}",
                    repo_owner,
                    repo_name,
                    repo_tree,
                    specific_file_contents,
                    issue_text,
                    lead_res.architectural_plan
                )
            } else {
                format!(
                    "Project Context: '{}/{}'.\n{}\n{}\nIssue: {}\nArchitectural Plan: {}\nREVIEWER FEEDBACK TO FIX: {}",
                    repo_owner,
                    repo_name,
                    repo_tree,
                    specific_file_contents,
                    issue_text,
                    lead_res.architectural_plan,
                    feedback_history
                )
            };

            let dev_raw = llm_client::ask(&dev_prompt, &dev_input).await?;

            let dev_res: DeveloperResponse = match serde_json::from_str(&dev_raw) {
                Ok(res) => res,
                Err(e) => {
                    println!("LLM generated invalid JSON: {}. Forcing retry...", e);

                    feedback_history = format!(
                        "CRITICAL SYSTEM ERROR: Your last response was NOT valid JSON. Error: '{}'. Strictly follow JSON formatting.",
                        e
                    );

                    attempt += 1;
                    if attempt > max_attempts {
                        break;
                    }
                    continue;
                }
            };

            println!("Dev Thought: {}", dev_res.thought_process);

            let is_lazy = dev_res.files_to_modify.iter().any(|f| {
                let content = &f.new_content;
                content.contains("// TODO")
                    || content.contains("/* TODO")
                    || content.contains("... existing code ...")
                    || content.contains("FIXME")
            });

            if is_lazy {
                println!(
                    "❌ SYSTEM REJECT: Agent tried to use placeholders (TODO/...). Forcing retry."
                );
                feedback_history = "CRITICAL ERROR: You left placeholders like '// TODO' or '... existing code ...' in the code. You MUST write the COMPLETE, ready-to-run code. Do not skip any logic. Write the ENTIRE file content.".to_string();
                attempt += 1;
                if attempt > max_attempts {
                    println!("🚨 MAX ATTEMPTS REACHED due to Agent Laziness.");
                    break;
                }
                continue;
            }

            git_local::create_branch(&dev_res.branch_name)?;
            file_system::apply_modifications(dev_res.files_to_modify)?;

            let commit_msg = format!(
                "feat: Resolve issue #{} (Attempt {})",
                target_issue.number, attempt
            );
            let _ = git_local::commit_changes(&commit_msg);

            println!(" Reviewer is analyzing the code...");
            let reviewer_prompt = fs::read_to_string("config/reviewer.md")?;
            let git_diff = git_local::get_diff_against_main().unwrap_or_default();
            let reviewer_input = format!(
                "Original Issue: {}\nArchitectural Plan (Scope): {}\nHere is the git diff for the new feature:\n{}",
                issue_text, lead_res.architectural_plan, git_diff
            );

            let rev_raw = llm_client::ask(&reviewer_prompt, &reviewer_input).await?;
            let rev_res: ReviewerResponse =
                serde_json::from_str(&rev_raw).expect("Failed to parse Reviewer JSON");

            if rev_res.is_approved {
                println!(" REVIEW APPROVED: Code is clean and production-ready!");
                git_local::push_to_remote(&dev_res.branch_name)?;

                println!(" Opening Pull Request on GitHub...");
                let pr_title = format!("Resolve Issue #{} - Auto AI PR", target_issue.number);
                let pr_body = format!(
                    " **Automated PR by AI Software Factory**\n**Agent Thought Process:** {}\n**Reviewer Notes:** Approved after {} attempts.\nCloses #{}",
                    dev_res.thought_process, attempt, target_issue.number
                );

                match github::create_pull_request(&pr_title, &pr_body, &dev_res.branch_name, "main")
                    .await
                {
                    Ok(url) => println!(" BOOM! Pull Request opened successfully: {}", url),
                    Err(e) => println!(" Failed to open PR: {}", e),
                }

                println!("🧹 Cleaning up workspace and returning to main...");
                let _ = git_local::reset_to_main();

                let issue_body = target_issue.body.clone().unwrap_or_default();

                // YENİ: Issue yorumlarını da çek
                let comments_history = github::fetch_issue_comments(target_issue.number)
                    .await
                    .unwrap_or_default();

                let issue_text = format!(
                    "Title: {}\nBody: {}\n\n--- COMMENTS HISTORY ---\n{}",
                    target_issue.title, issue_body, comments_history
                );

                println!(
                    " ======================================== Processing Issue #{}: {} ========================================",
                    target_issue.number, target_issue.title
                );

                println!("Reading repository tree...");
                let repo_tree = file_system::get_repo_tree();

                let team_lead_prompt = fs::read_to_string("config/team_lead.md")?;
                let team_lead_input = format!("{}  Issue Context: {}", repo_tree, issue_text);

                let lead_raw = llm_client::ask(&team_lead_prompt, &team_lead_input).await?;
                let mut lead_res: TeamLeaderResponse =
                    serde_json::from_str(&lead_raw).expect("Failed to parse Team Leader JSON");

                // --- YENİ: YORUM (COMMENT) TABANLI AUTO-CONTINUE MANTILI ---
                let mut was_truncated = false;
                let mut remaining_files = Vec::new();

                if lead_res.files_to_read.len() > 2 {
                    was_truncated = true;
                    remaining_files = lead_res.files_to_read.split_off(2); // İlk 2'yi tut, kalanları ayır

                    println!(
                        "⚠️ Token protection active! Retained 2 files. Remaining {} files saved for next cycle.",
                        remaining_files.len()
                    );

                    lead_res.architectural_plan = format!(
                        "{} [SYSTEM OVERRIDE: Due to token limits, ONLY modify these specific files in this PR: {:?}. Ignore the rest for now.]",
                        lead_res.architectural_plan, lead_res.files_to_read
                    );
                }

                let specific_file_contents =
                    file_system::read_specific_files(lead_res.files_to_read.clone());
                let dev_prompt =
                    fs::read_to_string(format!("config/{}.md", lead_res.assigned_agent))
                        .unwrap_or(fs::read_to_string("config/backend_dev.md")?);

                let mut feedback_history = String::new();
                let mut attempt = 1;
                let mut is_successful = false;

                loop {
                    println!(
                        " [ATTEMPT {}/3] {} is writing/fixing code...",
                        attempt, lead_res.assigned_agent
                    );

                    let dev_input = format!(
                        "Context: {}\nFiles: {}\nIssue: {}\nPlan: {}\nFeedback: {}",
                        std::env::var("GITHUB_REPO").unwrap_or_default(),
                        repo_tree,
                        specific_file_contents,
                        lead_res.architectural_plan,
                        feedback_history
                    );

                    let dev_raw = llm_client::ask(&dev_prompt, &dev_input).await?;
                    let dev_res: DeveloperResponse = match serde_json::from_str(&dev_raw) {
                        Ok(res) => res,
                        Err(e) => {
                            feedback_history = format!(
                                "Invalid JSON: {}. Fix formatting and escape characters.",
                                e
                            );
                            attempt += 1;
                            if attempt > 3 {
                                break;
                            }
                            continue;
                        }
                    };

                    // Laziness Check
                    if dev_res
                        .files_to_modify
                        .iter()
                        .any(|f| f.new_content.contains("// TODO") || f.new_content.contains("..."))
                    {
                        feedback_history =
                            "STOP! You used placeholders. Write the FULL file content.".to_string();
                        attempt += 1;
                        if attempt > 3 {
                            break;
                        }
                        continue;
                    }

                    let _ = git_local::create_branch(&dev_res.branch_name);
                    let _ = file_system::apply_modifications(dev_res.files_to_modify);
                    let _ = git_local::commit_changes(&format!(
                        "feat: Resolve #{}",
                        target_issue.number
                    ));

                    let reviewer_prompt = fs::read_to_string("config/reviewer.md")?;
                    let git_diff = git_local::get_diff_against_main().unwrap_or_default();

                    // Reviewer'a Kapsam bilgisini yolluyoruz
                    let reviewer_input = format!(
                        "Original Issue: {}\nArchitectural Plan (Scope): {}\nDiff:\n{}",
                        issue_text, lead_res.architectural_plan, git_diff
                    );

                    let rev_raw = llm_client::ask(&reviewer_prompt, &reviewer_input).await?;
                    let rev_res: ReviewerResponse =
                        serde_json::from_str(&rev_raw).expect("Rev JSON Failed");

                    if rev_res.is_approved {
                        let _ = git_local::push_to_remote(&dev_res.branch_name);
                        let _ = github::create_pull_request(
                            &format!("Fix #{} - Part", target_issue.number),
                            &dev_res.thought_process,
                            &dev_res.branch_name,
                            "main",
                        )
                        .await;
                        println!(" PR Opened!");
                        is_successful = true;
                        break;
                    } else {
                        feedback_history = rev_res.feedback_thread.unwrap_or_default();
                        attempt += 1;
                        if attempt > 3 {
                            break;
                        }
                    }
                }

                let _ = git_local::reset_to_main();

                if is_successful {
                    if was_truncated {
                        let comment = format!(
                            "**[AUTO-CONTINUE] Partial Completion** 🔄\nI have successfully updated these files in the latest PR: `{:?}`.\n\nDue to cognitive token limits, I still need to process the following files: `{:?}`.\n\n*I will pick this up in the next cycle automatically.*",
                            lead_res.files_to_read, remaining_files
                        );
                        let _ = github::create_issue_comment(target_issue.number, &comment).await;
                        println!(
                            " ⏳ Issue #{} partially completed. Auto-continue comment added.",
                            target_issue.number
                        );
                    } else {
                        processed_issues.insert(target_issue.number);
                        println!(
                            " ✅ Workflow fully completed for Issue #{}. Adding to memory.",
                            target_issue.number
                        );
                    }
                } else {
                    processed_issues.insert(target_issue.number);
                    println!(
                        " ❌ Workflow FAILED for Issue #{}. Agent could not complete the task.",
                        target_issue.number
                    );
                }

                sleep(Duration::from_secs(10)).await;
                let workspace = std::env::var("WORKSPACE_DIR").unwrap_or_default();
                let _ = std::process::Command::new("git")
                    .current_dir(&workspace)
                    .args(["branch", "-D", &dev_res.branch_name])
                    .output();

                break;
            } else {
                println!("❌ REVIEW REJECTED: Changes required.");
                if let Some(feedback) = rev_res.feedback_thread {
                    println!(" Feedback: {}", feedback);
                    feedback_history = feedback.clone();

                    println!(" Posting Reviewer feedback to GitHub Issue...");
                    let comment_body = format!(
                        "** Reviewer Feedback (Attempt {}):**\n{}",
                        attempt, feedback_history
                    );
                    let _ = github::create_issue_comment(target_issue.number, &comment_body).await;
                }

                attempt += 1;
                if attempt > max_attempts {
                    println!(" MAX ATTEMPTS REACHED! Halting feedback loop.");
                    let _ = git_local::reset_to_main();
                    break;
                }
            }
        }

        processed_issues.insert(target_issue.number);
        if is_successful {
            println!(
                " ✅ Workflow completed successfully for Issue #{}. Adding to memory.",
                target_issue.number
            );
        } else {
            println!(
                " ❌ Workflow FAILED for Issue #{}. Agent could not complete the task.",
                target_issue.number
            );
        }

        sleep(Duration::from_secs(10)).await;
    }
}
