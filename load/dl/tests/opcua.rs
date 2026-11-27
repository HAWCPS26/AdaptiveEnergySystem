use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use deferrable_load::{DeferrableLoadServer, LoadConfig, NAMESPACE_URI};
use opcua::{
    client::{ClientBuilder, IdentityToken, Session},
    crypto::SecurityPolicy,
    types::{
        CallMethodResult, DateTime as OpcUaDateTime, MessageSecurityMode, NodeId, ReadValueId,
        StatusCode, TimestampsToReturn, UserTokenPolicy, Variant,
    },
};
use tickgen::{ServerConfig, TickGenServer};
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn external_controller_can_drive_the_washing_machine(
) -> Result<(), Box<dyn std::error::Error>> {
    let tick_listener = TcpListener::bind("127.0.0.1:0").await?;
    let tick_address = tick_listener.local_addr()?;
    let load_listener = TcpListener::bind("127.0.0.1:0").await?;
    let load_address = load_listener.local_addr()?;

    let tick_url = format!("opc.tcp://{tick_address}/AdaptiveEnergySystem/TickGen");
    let load_url = format!("opc.tcp://{load_address}/AdaptiveEnergySystem/DeferrableLoad");

    let tick_pki = tempdir::TempDir::new("wash-tick-pki")?;
    let load_pki = tempdir::TempDir::new("wash-load-pki")?;
    let client_pki = tempdir::TempDir::new("wash-client-pki")?;
    let load_client_pki = tempdir::TempDir::new("wash-load-client-pki")?;
    let tick_client_pki = tempdir::TempDir::new("wash-tick-client-pki")?;

    let tick_config = ServerConfig {
        host: "127.0.0.1".to_owned(),
        port: tick_address.port(),
        discovery_url: Some(tick_url.clone()),
        pki_dir: tick_pki.path().to_owned(),
        tick_interval: Duration::from_millis(250),
        ..Default::default()
    };
    let tick_server = TickGenServer::new(tick_config)?;
    let tick_handle = tick_server.handle();
    let tick_task = tokio::spawn(tick_server.run_with_listener(tick_listener));

    let load_config = LoadConfig {
        host: "127.0.0.1".to_owned(),
        port: load_address.port(),
        discovery_url: Some(load_url.clone()),
        pki_dir: load_pki.path().to_owned(),
        client_pki_dir: load_client_pki.path().to_owned(),
        tickgen_url: tick_url.clone(),
        ..Default::default()
    };
    let mut load_server = DeferrableLoadServer::new(load_config.clone())?;
    let load_handle = load_server.handle();
    load_server.spawn_sync(load_config);
    let load_task = tokio::spawn(load_server.run_with_listener(load_listener));

    let mut client = ClientBuilder::new()
        .application_name("Washing Machine Integration Test")
        .application_uri("urn:AdaptiveEnergySystem:WashingMachineTest")
        .product_uri("urn:AdaptiveEnergySystem:WashingMachineTest")
        .trust_server_certs(true)
        .create_sample_keypair(true)
        .pki_dir(client_pki.path())
        .session_retry_limit(3)
        .client()
        .map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;

    let mut connected = None;
    for attempt in 0..50 {
        match client
            .connect_to_matching_endpoint(
                (
                    load_url.as_str(),
                    SecurityPolicy::None.to_str(),
                    MessageSecurityMode::None,
                    UserTokenPolicy::anonymous(),
                ),
                IdentityToken::Anonymous,
            )
            .await
        {
            Ok(connection) => {
                connected = Some(connection);
                break;
            }
            Err(_) if attempt < 49 => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let (session, event_loop) = connected.ok_or("connection attempts exhausted")?;
    let event_task = event_loop.spawn();
    assert!(session.wait_for_connection().await);
    let ns = session.get_namespace_index(NAMESPACE_URI).await?;

    // TickGen stays paused until the test explicitly starts it. Wait until the
    // appliance has subscribed, so no ticks can be missed.
    wait_for_bool(
        &session,
        ns,
        "TickGenConnected",
        true,
        Duration::from_secs(10),
    )
    .await?;

    assert_eq!(read_string(&session, ns, "State").await?, "Idle");
    assert!(!read_bool(&session, ns, "DeadlineSet").await?);

    // Starting without a deadline is rejected.
    let result = call_method(&session, ns, "Start", None).await?;
    assert_eq!(result.status_code, StatusCode::BadInvalidState);

    // The external EMS sets the deadline and starts the cycle.
    let deadline = Utc::now() + ChronoDuration::hours(8);
    let result = call_method(
        &session,
        ns,
        "SetDeadline",
        Some(vec![Variant::DateTime(Box::new(OpcUaDateTime::from(
            deadline,
        )))]),
    )
    .await?;
    assert!(result.status_code.is_good());
    assert!(read_bool(&session, ns, "DeadlineSet").await?);

    let result = call_method(&session, ns, "Start", None).await?;
    assert!(result.status_code.is_good());
    assert_eq!(read_string(&session, ns, "State").await?, "Running");

    // The external controller now starts the simulated clock.
    let mut tick_client = ClientBuilder::new()
        .application_name("Washing Machine Test TickGen Client")
        .application_uri("urn:AdaptiveEnergySystem:WashingMachineTest:TickGen")
        .product_uri("urn:AdaptiveEnergySystem:WashingMachineTest:TickGen")
        .trust_server_certs(true)
        .create_sample_keypair(true)
        .pki_dir(tick_client_pki.path())
        .session_retry_limit(3)
        .client()
        .map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;

    let mut tick_connected = None;
    for attempt in 0..50 {
        match tick_client
            .connect_to_matching_endpoint(
                (
                    tick_url.as_str(),
                    SecurityPolicy::None.to_str(),
                    MessageSecurityMode::None,
                    UserTokenPolicy::anonymous(),
                ),
                IdentityToken::Anonymous,
            )
            .await
        {
            Ok(connection) => {
                tick_connected = Some(connection);
                break;
            }
            Err(_) if attempt < 49 => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let (tick_session, tick_event_loop) =
        tick_connected.ok_or("tickgen connection attempts exhausted")?;
    let tick_event_task = tick_event_loop.spawn();
    assert!(tick_session.wait_for_connection().await);
    let tick_ns = tick_session.get_namespace_index(NAMESPACE_URI).await?;
    let result = tick_session
        .call_one((
            NodeId::new(tick_ns, "TickGenerator"),
            NodeId::new(tick_ns, "TickGenerator.Start"),
            Some(vec![Variant::UInt64(24)]),
        ))
        .await?;
    assert!(result.status_code.is_good());

    wait_for_state(&session, ns, "Done", Duration::from_secs(10)).await?;

    let energy = read_double(&session, ns, "EnergyDeliveredKwh").await?;
    assert!((energy - 0.9875).abs() < 1e-6, "energy was {energy}");
    assert!(read_bool(&session, ns, "DeadlineMet").await?);
    assert!((read_double(&session, ns, "ProgressPercent").await? - 100.0).abs() < 1e-9);
    assert_eq!(read_double(&session, ns, "CurrentPowerKw").await?, 0.0);

    // Pause, resume and stop are available from the outside after completion
    // as well, so the machine can be driven without reconnecting.
    let result = call_method(
        &session,
        ns,
        "SetDeadline",
        Some(vec![Variant::DateTime(Box::new(OpcUaDateTime::from(
            Utc::now() + ChronoDuration::hours(8),
        )))]),
    )
    .await?;
    assert!(result.status_code.is_good());
    assert!(call_method(&session, ns, "Start", None)
        .await?
        .status_code
        .is_good());
    assert!(call_method(&session, ns, "Pause", None)
        .await?
        .status_code
        .is_good());
    assert_eq!(read_string(&session, ns, "State").await?, "Paused");
    assert!(call_method(&session, ns, "Resume", None)
        .await?
        .status_code
        .is_good());
    assert_eq!(read_string(&session, ns, "State").await?, "Running");
    assert!(call_method(&session, ns, "Stop", None)
        .await?
        .status_code
        .is_good());
    assert_eq!(read_string(&session, ns, "State").await?, "Stopped");
    assert_eq!(read_double(&session, ns, "CycleElapsedMinutes").await?, 0.0);

    session.disconnect().await?;
    event_task.await?;
    tick_session.disconnect().await?;
    tick_event_task.await?;
    load_handle.cancel();
    load_task.await??;
    tick_handle.cancel();
    tick_task.await??;
    Ok(())
}

async fn call_method(
    session: &Session,
    ns: u16,
    name: &str,
    arguments: Option<Vec<Variant>>,
) -> Result<CallMethodResult, Box<dyn std::error::Error>> {
    let result = session
        .call_one((
            NodeId::new(ns, "DeferrableLoad"),
            NodeId::new(ns, format!("DeferrableLoad.{name}")),
            arguments,
        ))
        .await?;
    Ok(result)
}

async fn read_value(
    session: &Session,
    ns: u16,
    name: &str,
) -> Result<Variant, Box<dyn std::error::Error>> {
    let node: ReadValueId = NodeId::new(ns, format!("DeferrableLoad.{name}")).into();
    let values = session
        .read(&[node], TimestampsToReturn::Neither, 0.0)
        .await?;
    values
        .into_iter()
        .next()
        .and_then(|value| value.value)
        .ok_or_else(|| "the node returned no value".into())
}

async fn read_string(
    session: &Session,
    ns: u16,
    name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match read_value(session, ns, name).await? {
        Variant::String(value) => Ok(value.to_string()),
        value => Err(format!("{name} was not a string: {value:?}").into()),
    }
}

async fn read_bool(
    session: &Session,
    ns: u16,
    name: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    match read_value(session, ns, name).await? {
        Variant::Boolean(value) => Ok(value),
        value => Err(format!("{name} was not a boolean: {value:?}").into()),
    }
}

async fn read_double(
    session: &Session,
    ns: u16,
    name: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    match read_value(session, ns, name).await? {
        Variant::Double(value) => Ok(value),
        value => Err(format!("{name} was not a double: {value:?}").into()),
    }
}

async fn wait_for_state(
    session: &Session,
    ns: u16,
    expected: &str,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let state = read_string(session, ns, "State").await?;
        if state == expected {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("state was {state}, expected {expected}").into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_bool(
    session: &Session,
    ns: u16,
    name: &str,
    expected: bool,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let value = read_bool(session, ns, name).await?;
        if value == expected {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("{name} was {value}, expected {expected}").into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
