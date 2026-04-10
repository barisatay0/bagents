use dotenv::dotenv;

mod clients;
mod helpers;
mod models;
mod orchestrator;
mod services;

use env_logger::init;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    dotenv().ok();
    info!("========================================");
    info!("BAGENTS: Autonomous Software Factory ");
    info!("========================================\
");

    // Start the complete autonomous factory workflow
    if let Err(e) = orchestrator::run_factory().await {
        error!("\\\u{1F6AB} Factory encountered an error: {}", e);
    }

    Ok(())
}
