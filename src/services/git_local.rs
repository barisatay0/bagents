use std::process::Command;
use tracing::{debug, info, warn};

use crate::config::Config;

/// Checkout a branch. If it exists locally or on the remote, it switches to it and pulls the latest.
/// If it doesn't exist, it creates a fresh branch from main.
pub fn setup_branch(config: &Config, branch_name: &str) -> Result<(), String> {
    let _ = run(config, &["fetch", "origin"]);

    let checkout_out = run(config, &["checkout", branch_name])?;

    if checkout_out.status.success() {
        info!(
            branch = branch_name,
            "Switched to existing branch — resuming work"
        );
        let _ = run(config, &["pull", "origin", branch_name]);
        Ok(())
    } else {
        info!(branch = branch_name, "Creating new branch");
        let create_out = run(config, &["checkout", "-b", branch_name])?;
        if !create_out.status.success() {
            return Err(format!(
                "Failed to create branch: {}",
                String::from_utf8_lossy(&create_out.stderr)
            ));
        }
        Ok(())
    }
}

/// Stage all changes and commit with the given message.
/// Succeeds silently when there is nothing to commit.
pub fn commit_changes(config: &Config, message: &str) -> Result<(), String> {
    debug!("Staging and committing changes");

    run(config, &["add", "."])?;

    let out = run(config, &["commit", "-m", message])?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();

    if !out.status.success() && !stdout.contains("nothing to commit") {
        return Err(format!(
            "git commit failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// Return the unified diff of the current branch against `main`.
pub fn get_diff_against_main(config: &Config) -> Result<String, String> {
    let out = run(config, &["diff", "-U2", "main"])?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Force-push the branch to `origin` using the tracked remote.
pub fn push_to_remote(config: &Config, branch_name: &str) -> Result<(), String> {
    info!(branch = branch_name, "Pushing branch to remote");

    let out = run(config, &["push", "-u", "origin", branch_name])?;
    if !out.status.success() {
        return Err(format!(
            "git push failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

/// Hard-reset and checkout main, then pull latest from origin.
pub fn reset_to_main(config: &Config) -> Result<(), String> {
    info!("Resetting workspace to main branch");

    run(config, &["reset", "--hard"])?;
    run(config, &["checkout", "main"])?;

    let out = run(config, &["pull", "origin", "main"])?;
    if !out.status.success() {
        warn!(
            stderr = %String::from_utf8_lossy(&out.stderr),
            "git pull returned non-zero — workspace may be stale"
        );
    }
    Ok(())
}

/// Delete a local branch by name, ignoring errors (best-effort cleanup).
pub fn delete_local_branch(config: &Config, branch_name: &str) {
    debug!(branch = branch_name, "Deleting local branch");
    let _ = run(config, &["branch", "-D", branch_name]);
}

// ── internal helpers ─────────────────────────────────────────────────────────

fn run(config: &Config, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .current_dir(&config.workspace_dir)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to spawn git {:?}: {}", args, e))
}

pub fn reset_working_tree(config: &Config) -> Result<(), String> {
    let _ = run(config, &["reset", "--hard", "HEAD"]);
    let _ = run(config, &["clean", "-fd"]);
    Ok(())
}

pub fn run_verification(config: &Config) -> Result<(), String> {
    if config.verify_command.trim().is_empty() {
        return Ok(());
    }

    info!("Running verification command: {}", config.verify_command);

    let parts: Vec<&str> = config.verify_command.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }

    let out = Command::new(parts[0])
        .args(&parts[1..])
        .current_dir(&config.workspace_dir)
        .output()
        .map_err(|e| format!("Failed to run verification command: {}", e))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);

        let err_str = if combined.len() > 2000 {
            format!("...{}", &combined[combined.len() - 2000..])
        } else {
            combined
        };
        return Err(err_str);
    }

    Ok(())
}
