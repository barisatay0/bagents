use dotenvy::dotenv;
use env_logger::Env;
use log::*;

async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env before anything else so env vars are available for config
    dotenv().ok();

    // Initialize logging system
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    info!("Starting BAGENTS factory system");

    println!("========================================");
    println!("BAGENTS: Autonomous Software Factory");
    println!("========================================
");

    // Validate all config and prompt files at startup — fail fast with
    // a clear message rather than panicking mid-run.
    let config = Config::from_env().map_err(|e| {
        error!("Failed to load configuration: {}", e);
        e
    })?;

    let prompts = Prompts::load().map_err(|e| {
        error!("Failed to load prompts: {}", e);
        e
    })?;

    info!(
        owner = %config.github_owner,
        repo = %config.github_repo,
        workspace = %config.workspace_dir.display(),
        model = %config.llm_model,
        "Configuration loaded successfully"
    );

    if let Err(e) = orchestrator::run_factory(&config, &prompts).await {
        error!(err = %e, "Factory encountered a fatal error");
    }

    Ok(())
}