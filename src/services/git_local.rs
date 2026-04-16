use std::process::Command;
use tracing::{debug, info, warn};

use crate::config::Config;

/// Checkout a new branch in the workspace, or switch to it if it already exists.
pub fn create_branch(config: &Config, branch_name: &str) -> Result<(), String> {
    info!(branch = branch_name, "Creating git branch");

    let out = run(config, &["checkout", "-b", branch_name])?;
    if !out.status.success() {
        // Branch already exists — just switch to it
        run(config, &["checkout", branch_name])?;
    }
    Ok(())
}

/// Stage all changes and commit with the given message.
/// Succeeds silently when there is nothing to commit.
pub fn commit_changes(config: &Config, message: &str) -> Result<(), String> {
    info!("Initiating git commit process: Staging all changes for commit");
    debug!("Staging and committing changes");

    run(config, &["add", "."])?;

    info!("Executing git commit with message: '{}'", message);
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
