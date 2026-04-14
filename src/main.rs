use dotenv::dotenv;
use log::{info, error};

mod clients;
mod helpers;
mod models;
mod orchestrator;
mod services;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    env_logger::init();
    dotenv().ok();
    info!("========================================");
    info!("BAGENTS: Autonomous Software Factory ");
    info!("========================================
");

    // Start the complete autonomous factory workflow
    if let Err(e) = orchestrator::run_factory().await {
        error!("❌ Factory encountered an error: {}", e);
    }

    Ok(())
}
