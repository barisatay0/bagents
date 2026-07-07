use std::process::Command;
use tracing::{debug, info, warn};

use crate::config::Config;

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

pub fn get_diff_against_base(config: &Config) -> Result<String, String> {
    let out = run(config, &["diff", "-U2", &config.base_branch])?;
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

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

pub fn reset_to_base(config: &Config) -> Result<(), String> {
    info!("Resetting workspace to {} branch", config.base_branch);

    run(config, &["reset", "--hard"])?;
    run(config, &["checkout", &config.base_branch])?;

    let out = run(config, &["pull", "origin", &config.base_branch])?;
    if !out.status.success() {
        warn!(
            stderr = %String::from_utf8_lossy(&out.stderr),
            "git pull returned non-zero — workspace may be stale"
        );
    }
    Ok(())
}

pub fn delete_local_branch(config: &Config, branch_name: &str) {
    debug!(branch = branch_name, "Deleting local branch");
    let _ = run(config, &["branch", "-D", branch_name]);
}

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

    let out = Command::new("sh")
        .arg("-c")
        .arg(&config.verify_command)
        .current_dir(&config.workspace_dir)
        .output()
        .map_err(|e| format!("Failed to run verification command: {}", e))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let combined = format!("{}\n{}", stdout, stderr);

        let diagnostics = parse_diagnostics(&combined, &config.verify_command);
        return Err(diagnostics);
    }

    Ok(())
}

#[derive(Debug)]
struct Diagnostic {
    file_path: String,
    line: Option<u32>,
    message: String,
}

const MAX_DIAGNOSTICS: usize = 10;

fn parse_diagnostics(raw: &str, verify_command: &str) -> String {
    let is_cargo = verify_command.contains("cargo");

    let diagnostics = if is_cargo {
        parse_cargo_diagnostics(raw)
    } else {
        parse_generic_diagnostics(raw)
    };

    if diagnostics.is_empty() {
        let trimmed = raw.trim();
        let snippet = if trimmed.chars().count() > 1500 {
            let start_char_idx = trimmed.chars().count() - 1500;
            let byte_idx = trimmed.char_indices().nth(start_char_idx).map(|(idx, _)| idx).unwrap_or(0);
            format!("…[truncated]\n{}", &trimmed[byte_idx..])
        } else {
            trimmed.to_string()
        };
        return format!("Verification failed. Raw output:\n{}", snippet);
    }

    let shown: Vec<&Diagnostic> = diagnostics.iter().take(MAX_DIAGNOSTICS).collect();
    let total = diagnostics.len();
    let omitted = total.saturating_sub(MAX_DIAGNOSTICS);

    let mut out = format!(
        "Verification failed — {} error(s) found{}:\n\n",
        total,
        if omitted > 0 { format!(" ({} shown)", shown.len()) } else { String::new() }
    );

    for d in &shown {
        match d.line {
            Some(l) => out.push_str(&format!("  {}:{} — {}\n", d.file_path, l, d.message)),
            None    => out.push_str(&format!("  {} — {}\n", d.file_path, d.message)),
        }
    }

    if omitted > 0 {
        out.push_str(&format!(
            "\n  … and {} more error(s) not shown. Fix the above first.\n",
            omitted
        ));
    }

    out.push_str("\nFix ONLY the errors in files you modified. Do not change unrelated code.");
    out
}

fn parse_cargo_diagnostics(raw: &str) -> Vec<Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let lines: Vec<&str> = raw.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim_start();

        if (line.starts_with("error[") || line.starts_with("error:"))
            && !line.contains("aborting due to")
            && !line.contains("could not compile")
            && !line.starts_with("error: cannot open")
        {
            let message = line
                .trim_start_matches("error")
                .trim_start_matches(|c: char| c == '[' || c.is_ascii_alphanumeric() || c == ']')
                .trim_start_matches(':')
                .trim()
                .to_string();

            let mut file_path = String::new();
            let mut diag_line: Option<u32> = None;

            for candidate_line in lines.iter().skip(i + 1).take(5) {
                let candidate = candidate_line.trim();
                if let Some(rest) = candidate.strip_prefix("-->") {
                    let loc = rest.trim();
                    let mut parts = loc.splitn(3, ':');
                    if let Some(fp) = parts.next() {
                        file_path = fp.trim().to_string();
                    }
                    if let Some(ln) = parts.next() {
                        diag_line = ln.trim().parse::<u32>().ok();
                    }
                    break;
                }
            }

            if !file_path.is_empty() {
                diagnostics.push(Diagnostic {
                    file_path,
                    line: diag_line,
                    message: if message.is_empty() { "error".to_string() } else { message },
                });
            }
        }

        i += 1;
    }

    diagnostics
}

fn parse_generic_diagnostics(raw: &str) -> Vec<Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty()
            || trimmed.starts_with("Found ")
            || trimmed.starts_with("npm ")
            || trimmed.starts_with("warning")
            || trimmed.starts_with("Warning")
        {
            continue;
        }

        if !trimmed.to_lowercase().contains("error") {
            continue;
        }

        let parts: Vec<&str> = trimmed.splitn(4, ':').collect();
        if parts.len() >= 3 {
            let file_candidate = parts[0].trim();
            let line_candidate = parts[1].trim();

            let looks_like_path = file_candidate.contains('/')
                || file_candidate.contains('.')
                || file_candidate.starts_with("src")
                || file_candidate.starts_with("lib")
                || file_candidate.starts_with("tests");

            if looks_like_path
                && let Ok(line_no) = line_candidate.parse::<u32>()
            {
                    let raw_msg = parts[2..].join(":").trim().to_string();
                    let message = {
                        let segments: Vec<&str> = raw_msg.splitn(2, ':').collect();
                        if segments.len() == 2 && segments[0].trim().parse::<u32>().is_ok() {
                            segments[1].trim().to_string()
                        } else {
                            raw_msg
                        }
                    };

                    diagnostics.push(Diagnostic {
                        file_path: file_candidate.to_string(),
                        line: Some(line_no),
                        message,
                    });
                    continue;
            }
        }

        if trimmed.len() < 200 {
            diagnostics.push(Diagnostic {
                file_path: String::new(),
                line: None,
                message: trimmed.to_string(),
            });
        }
    }

    let has_file_errors = diagnostics.iter().any(|d| !d.file_path.is_empty());
    if has_file_errors {
        diagnostics.retain(|d| !d.file_path.is_empty());
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cargo_diagnostics() {
        let err_output = r#"
   Compiling my_project v0.1.0 (/workspace)
error[E0308]: mismatched types
  --> src/main.rs:10:15
   |
10 |     let x: u32 = "hello";
   |            ---   ^^^^^^^ expected `u32`, found `&str`
   |            |
   |            expected due to this
        "#;
        let parsed = parse_cargo_diagnostics(err_output);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].file_path, "src/main.rs");
        assert_eq!(parsed[0].line, Some(10));
        assert_eq!(parsed[0].message, "mismatched types");
    }

    #[test]
    fn test_parse_generic_diagnostics() {
        let err_output = r#"
src/app.js:20:10: error: Unexpected identifier
        "#;
        let parsed = parse_generic_diagnostics(err_output);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].file_path, "src/app.js");
        assert_eq!(parsed[0].line, Some(20));
        assert_eq!(parsed[0].message, "error: Unexpected identifier");
    }
}
