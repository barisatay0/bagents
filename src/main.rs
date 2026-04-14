use dotenv::dotenv;
use log::info;
use log::error;

mod clients;
mod helpers;
mod models;
mod orchestrator;
mod services;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    
    dotenv().ok();
    env_logger::init();
    info!("========================================");
    info!("BAGENTS: Autonomous Software Factory ");
    info!("========================================\n");
    
    // Start the complete autonomous factory workflow
    if let Err(e) = orchestrator::run_factory().await {
        error!("\u{274C} Factory encountered an error: {}", e);
    }
    
    Ok(())
}
