pub mod file_system;
pub mod git_local;
pub mod github;
pub mod gitlab;
pub mod forgejo;
pub mod jira;
pub mod semantic;
pub mod traits;

use crate::config::Config;
pub use traits::{IssueTracker, RepoService};

pub fn build_tracker(config: &Config) -> Result<Box<dyn IssueTracker>, Box<dyn std::error::Error>> {
    match config.tracker_type.as_str() {
        "github" => Ok(Box::new(github::GithubService::new(
            config.tracker_token.clone(),
            config.tracker_url.clone(),
            config.tracker_project.clone(),
        )?)),
        "gitlab" => Ok(Box::new(gitlab::GitlabService::new(
            config.tracker_token.clone(),
            config.tracker_url.clone(),
            config.tracker_project.clone(),
        )?)),
        "jira" => Ok(Box::new(jira::JiraService::new(
            config.tracker_token.clone(),
            config.tracker_username.clone(),
            config.tracker_url.clone(),
            config.tracker_project.clone(),
        )?)),
        _ => Err(format!("Unknown tracker_type: {}", config.tracker_type).into()),
    }
}

pub fn build_repo_service(config: &Config) -> Result<Box<dyn RepoService>, Box<dyn std::error::Error>> {
    match config.repo_type.as_str() {
        "github" => Ok(Box::new(github::GithubService::new(
            config.repo_token.clone(),
            config.repo_url.clone(),
            config.repo_project.clone(),
        )?)),
        "gitlab" => Ok(Box::new(gitlab::GitlabService::new(
            config.repo_token.clone(),
            config.repo_url.clone(),
            config.repo_project.clone(),
        )?)),
        "forgejo" => Ok(Box::new(forgejo::ForgejoService::new(
            config.repo_token.clone(),
            config.repo_url.clone(),
            config.repo_project.clone(),
        )?)),
        _ => Err(format!("Unknown repo_type: {}", config.repo_type).into()),
    }
}
