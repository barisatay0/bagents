use dotenv::dotenv;
use log::{info, error};

mod clients;
mod config;
mod helpers;
mod models;
mod orchestrator;
mod prompts;
mod services;

use config::Config;
use prompts::Prompts;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env before anything else so env vars are available for config
    dotenv().ok();

    // Initialize logger (env_logger) – control verbosity with RUST_LOG env var.
    // e.g. RUST_LOG=bagents=debug or RUST_LOG=info
    env_logger::init();

    info!("========================================");
    info!("BAGENTS: Autonomous Software Factory");
    info!("========================================
");

    // Validate all config and prompt files at startup — fail fast with
    // a clear message rather than panicking mid-run.
    let config = Config::from_env().map_err(|e| {
        error!("
{}
", e);
        e
    })?;

    let prompts = Prompts::load().map_err(|e| {
        error!("
{}
", e);
        e
    })?;

    info!(
        owner = %config.github_owner,
        repo = %config.github_repo,
        workspace = %config.workspace_dir.display(),
        model = %config.llm_model,
        "Configuration loaded"
    );

    if let Err(e) = orchestrator::run_factory(&config, &prompts).await {
        error!(err = %e, "Factory encountered a fatal error");
    }

    Ok(())
}
