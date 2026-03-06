use std::process::Command;

pub fn create_branch(branch_name: &str) -> Result<(), String> {
    println!("Creating git branch: {}", branch_name);
    let output = Command::new("git")
        .args(["checkout", "-b", branch_name])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        // If branch exists, just check it out
        Command::new("git")
            .args(["checkout", branch_name])
            .output()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn commit_changes(message: &str) -> Result<(), String> {
    println!("Committing changes...");
    Command::new("git")
        .args(["add", "."])
        .output()
        .map_err(|e| e.to_string())?;

    let output = Command::new("git")
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
    let output = Command::new("git")
        .args(["diff", "main"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
