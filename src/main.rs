use dotenv::dotenv;

mod clients;
mod helpers;
mod models;
mod orchestrator;
mod services;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    dotenv().ok();
    println!("========================================");
    println!("BAGENTS: Autonomous Software Factory ");
    println!("========================================\n");

    // Start the complete autonomous factory workflow
    if let Err(e) = orchestrator::run_factory().await {
        println!("❌ Factory encountered an error: {}", e);
    }

    Ok(())
}
