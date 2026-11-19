use tickgen::{ServerConfig, TickGenServer};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env()?;
    let start_ticks = config.start_ticks;
    let start_delay = config.start_delay;
    let endpoint_url = config.endpoint_url();
    let mut server = TickGenServer::new(config)?;

    if start_ticks > 0 {
        if start_delay.is_zero() {
            server.start(start_ticks)?;
            info!("Automatically started {start_ticks} ticks");
        } else {
            server.start_after(start_ticks, start_delay);
            info!("Starting {start_ticks} ticks in {start_delay:?}");
        }
    }

    info!("TickGen OPC UA endpoint: {endpoint_url}");
    server.run().await?;
    Ok(())
}
