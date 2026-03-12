use std::env;
use std::process::Command;

fn get_workspace() -> String {
    env::var("WORKSPACE_DIR").expect("WORKSPACE_DIR missing in .env")
}

pub fn create_branch(branch_name: &str) -> Result<(), String> {
    let workspace = get_workspace();
    println!(
        "Creating git branch '{}' in workspace: {}",
        branch_name, workspace
    );

    let output = Command::new("git")
        .current_dir(&workspace)
        .args(["checkout", "-b", branch_name])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        Command::new("git")
            .current_dir(&workspace)
            .args(["checkout", branch_name])
            .output()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn commit_changes(message: &str) -> Result<(), String> {
    let workspace = get_workspace();
    println!("Committing changes in workspace...");

    Command::new("git")
        .current_dir(&workspace)
        .args(["add", "."])
        .output()
        .map_err(|e| e.to_string())?;

    let output = Command::new("git")
        .current_dir(&workspace)
        .args(["commit", "-m", message])
        .output()
        .map_err(|e| e.to_string())?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
    if !output.status.success() && !stdout.contains("nothing to commit") {
        return Err(format!(
            "Git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

pub fn get_diff_against_main() -> Result<String, String> {
    let workspace = get_workspace();
    let output = Command::new("git")
        .current_dir(&workspace)
        .args(["diff", "main"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn push_to_remote(branch_name: &str) -> Result<(), String> {
    let workspace = get_workspace();
    println!(
        "Pushing branch '{}' from workspace to remote...",
        branch_name
    );

    let output = Command::new("git")
        .current_dir(&workspace)
        .args(["push", "-u", "origin", branch_name])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(format!(
            "Git push failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}
