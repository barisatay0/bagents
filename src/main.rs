use dotenv::dotenv;
use env_logger::init;

mod clients;
mod helpers;
mod models;
mod orchestrator;
mod services;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    dotenv().ok();
    init();
    log::info!("========================================");
    log::info!("BAGENTS: Autonomous Software Factory ");
    log::info!("========================================\n");

    // Start the complete autonomous factory workflow
    if let Err(e) = orchestrator::run_factory().await {
        log::error!("Factory encountered an error: {}", e);
    }

    Ok(())
}
