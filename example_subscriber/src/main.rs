use std::{env, time::Duration};

use opcua::{
    client::{ClientBuilder, DataChangeCallback, IdentityToken},
    crypto::SecurityPolicy,
    types::{
        MessageSecurityMode, MonitoredItemCreateRequest, NodeId, ReadValueId, TimestampsToReturn,
        UserTokenPolicy, Variant,
    },
};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const DEFAULT_URL: &str = "opc.tcp://127.0.0.1:4840/AdaptiveEnergySystem/TickGen";
const NAMESPACE_URI: &str = "urn:AdaptiveEnergySystem";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,opcua=warn")),
        )
        .init();

    let url = env::var("TICKGEN_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());
    let mut client = ClientBuilder::new()
        .application_name("TickGen Example Subscriber")
        .application_uri("urn:AdaptiveEnergySystem:ExampleSubscriber")
        .product_uri("urn:AdaptiveEnergySystem:ExampleSubscriber")
        .trust_server_certs(true)
        .create_sample_keypair(true)
        .session_retry_limit(-1)
        .client()
        .map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;

    // Discovery happens before the session retry loop, so retry it while Docker starts TickGen.
    let mut attempt = 0_u32;
    let (session, event_loop) = loop {
        match client
            .connect_to_matching_endpoint(
                (
                    url.as_str(),
                    SecurityPolicy::None.to_str(),
                    MessageSecurityMode::None,
                    UserTokenPolicy::anonymous(),
                ),
                IdentityToken::Anonymous,
            )
            .await
        {
            Ok(connection) => break connection,
            Err(error) if attempt < 30 => {
                attempt += 1;
                warn!("TickGen is not ready ({error}); retrying in one second");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(error) => return Err(error.into()),
        }
    };

    let mut event_handle = event_loop.spawn();
    if !session.wait_for_connection().await {
        return Err("Could not establish an OPC UA session".into());
    }
    let ns = session.get_namespace_index(NAMESPACE_URI).await?;
    info!("Connected to {url}");

    let (tick_tx, mut tick_rx) = mpsc::unbounded_channel::<u64>();
    let subscription_id = session
        .create_subscription(
            Duration::from_millis(100),
            10,
            30,
            0,
            0,
            true,
            DataChangeCallback::new(move |data_value, _| {
                if let Some(Variant::UInt64(id)) = data_value.value {
                    let _ = tick_tx.send(id);
                }
            }),
        )
        .await?;
    let items: Vec<MonitoredItemCreateRequest> =
        vec![NodeId::new(ns, "TickGenerator.CurrentTick").into()];
    session
        .create_monitored_items(subscription_id, TimestampsToReturn::Both, items)
        .await?;

    let read_nodes: Vec<ReadValueId> = ["SimulationTime", "Running", "TimeSinceLastTick"]
        .into_iter()
        .map(|name| NodeId::new(ns, format!("TickGenerator.{name}")).into())
        .collect();

    loop {
        tokio::select! {
            Some(tick) = tick_rx.recv() => {
                let values = session
                    .read(&read_nodes, TimestampsToReturn::Neither, 0.0)
                    .await?;
                if matches!(values[1].value, Some(Variant::Boolean(false))) {
                    continue;
                }
                println!(
                    "reacts to tick: tick={tick}, simulation_time={}, time_since_last_tick_ms={}",
                    display_value(&values[0].value),
                    display_value(&values[2].value),
                );
            }
            _ = tokio::signal::ctrl_c() => break,
            result = &mut event_handle => {
                result?;
                return Err("The OPC UA session ended".into());
            }
        }
    }

    session.disconnect().await?;
    Ok(())
}

fn display_value(value: &Option<Variant>) -> String {
    value
        .as_ref()
        .map_or_else(|| "<no value>".to_owned(), |value| value.to_string())
}
