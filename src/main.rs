use dotenv::dotenv;
use log::info;

mod clients;
mod helpers;
mod models;
mod orchestrator;
mod services;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    dotenv().ok();
    info!("Starting BAGENTS: Autonomous Software Factory with configuration: {{}}", "default");
    info!("========================================");
    info!("BAGENTS: Autonomous Software Factory ");
    info!("========================================\n");

    // Start the complete autonomous factory workflow
    if let Err(e) = orchestrator::run_factory().await {
        error!("Factory encountered an error: {}", e);
    }

    Ok(())
}
