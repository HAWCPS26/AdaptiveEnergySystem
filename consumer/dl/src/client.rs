use std::{path::Path, sync::Arc, time::Duration};

use crate::{LoadConfig, SharedLoad, NAMESPACE_URI};
use chrono::{DateTime, Utc};
use opcua::{
    client::{ClientBuilder, DataChangeCallback, IdentityToken, Session},
    crypto::SecurityPolicy,
    types::{
        MessageSecurityMode, MonitoredItemCreateRequest, NodeId, ReadValueId, StatusCode,
        TimestampsToReturn, UserTokenPolicy, Variant,
    },
};
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{info, warn};

const RECONNECT_DELAY: Duration = Duration::from_secs(1);

type ClientResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Follow TickGen and advance the washing machine once per tick.
pub(crate) fn spawn_sync(config: LoadConfig, shared: SharedLoad) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(error) = run_session(&config, &shared).await {
                warn!("Washing machine sync failed ({error}); retrying in one second");
            }
            shared.set_tickgen_connected(false);
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    })
}

async fn run_session(config: &LoadConfig, shared: &SharedLoad) -> ClientResult<()> {
    let (session, mut event_handle, ns) = connect(
        &config.tickgen_url,
        &config.client_pki_dir,
        "Adaptive Energy System Washing Machine TickGen Client",
        "urn:AdaptiveEnergySystem:WashingMachine:TickGenClient",
    )
    .await?;

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
                if let Some(Variant::UInt64(tick)) = data_value.value {
                    let _ = tick_tx.send(tick);
                }
            }),
        )
        .await?;
    let items: Vec<MonitoredItemCreateRequest> =
        vec![NodeId::new(ns, "TickGenerator.CurrentTick").into()];
    session
        .create_monitored_items(subscription_id, TimestampsToReturn::Both, items)
        .await?;
    shared.set_tickgen_connected(true);

    let read_nodes: Vec<ReadValueId> = ["SimulationTime", "SimulatedTickMinutes"]
        .into_iter()
        .map(|name| NodeId::new(ns, format!("TickGenerator.{name}")).into())
        .collect();

    info!("Washing machine connected to TickGen");
    let mut last_tick = None;
    loop {
        tokio::select! {
            maybe_tick = tick_rx.recv() => {
                let Some(tick) = maybe_tick else {
                    return Err(std::io::Error::other("tick subscription ended").into());
                };
                if tick == 0 || last_tick == Some(tick) {
                    continue;
                }
                last_tick = Some(tick);

                let values = session
                    .read(&read_nodes, TimestampsToReturn::Neither, 0.0)
                    .await?;
                let Some(simulation_time) =
                    values.first().and_then(|value| date_time(value.value.as_ref()))
                else {
                    warn!("TickGen returned no SimulationTime for tick {tick}");
                    continue;
                };
                let tick_minutes = values
                    .get(1)
                    .and_then(|value| match value.value.as_ref() {
                        Some(Variant::UInt32(minutes)) if *minutes > 0 => Some(*minutes),
                        _ => None,
                    })
                    .unwrap_or(15);

                let before = shared.snapshot();
                shared.on_tick(simulation_time, tick_minutes);
                let after = shared.snapshot();
                let changed =
                    before.state != after.state || before.phase != after.phase;

                if changed {
                    info!(
                        state = after.state.as_str(),
                        phase = after.phase,
                        progress_percent = after.progress_percent,
                        "washing machine state changed"
                    );
                }
                info!(
                    tick,
                    state = after.state.as_str(),
                    phase = after.phase,
                    progress_percent = after.progress_percent,
                    power_kw = after.current_power_kw,
                    energy_kwh = after.energy_delivered_kwh,
                    simulation_time = %simulation_time.to_rfc3339(),
                    "washing machine tick"
                );
            }
            result = &mut event_handle => {
                let reason = match result {
                    Ok(status) => format!("TickGen session ended with {status}"),
                    Err(error) => format!("TickGen event loop failed: {error}"),
                };
                return Err(std::io::Error::other(reason).into());
            }
        }
    }
}

async fn connect(
    url: &str,
    pki_dir: &Path,
    application_name: &str,
    application_uri: &str,
) -> ClientResult<(Arc<Session>, JoinHandle<StatusCode>, u16)> {
    let mut client = ClientBuilder::new()
        .application_name(application_name)
        .application_uri(application_uri)
        .product_uri(application_uri)
        .trust_server_certs(true)
        .create_sample_keypair(true)
        .pki_dir(pki_dir)
        .session_retry_limit(-1)
        .client()
        .map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;

    let mut attempt = 0_u32;
    let (session, event_loop) = loop {
        match client
            .connect_to_matching_endpoint(
                (
                    url,
                    SecurityPolicy::None.to_str(),
                    MessageSecurityMode::None,
                    UserTokenPolicy::anonymous(),
                ),
                IdentityToken::Anonymous,
            )
            .await
        {
            Ok(connection) => break connection,
            Err(error) if attempt < 10 => {
                attempt += 1;
                warn!("TickGen is not ready ({error}); retrying in one second");
                tokio::time::sleep(RECONNECT_DELAY).await;
            }
            Err(error) => return Err(error.into()),
        }
    };

    let event_handle = event_loop.spawn();
    if !session.wait_for_connection().await {
        event_handle.abort();
        return Err(std::io::Error::other("could not establish an OPC UA session").into());
    }
    let ns = session.get_namespace_index(NAMESPACE_URI).await?;
    Ok((session, event_handle, ns))
}

fn date_time(value: Option<&Variant>) -> Option<DateTime<Utc>> {
    match value {
        Some(Variant::DateTime(value)) => Some(value.as_chrono()),
        _ => None,
    }
}
