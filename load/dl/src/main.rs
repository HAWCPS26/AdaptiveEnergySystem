use deferrable_load::{DeferrableLoadServer, LoadConfig};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,opcua=warn")),
        )
        .init();

    let config = LoadConfig::from_env()?;
    let endpoint_url = config.endpoint_url();

    let mut server = DeferrableLoadServer::new(config.clone())?;
    server.spawn_sync(config);

    info!("Washing machine OPC UA endpoint: {endpoint_url}");
    server.run().await?;
    Ok(())
}
