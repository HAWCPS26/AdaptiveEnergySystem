use std::time::Duration;

use opcua::{
    client::{ClientBuilder, DataChangeCallback, IdentityToken, Session},
    crypto::SecurityPolicy,
    types::{
        CallMethodResult, MessageSecurityMode, NodeId, ReadValueId, StatusCode, TimestampsToReturn,
        UserTokenPolicy, Variant,
    },
};
use tickgen::{ServerConfig, TickGenServer, NAMESPACE_URI};
use tokio::{net::TcpListener, sync::mpsc};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn client_can_control_and_subscribe_to_ticks() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server_pki = tempdir::TempDir::new("tickgen-server-pki")?;
    let client_pki = tempdir::TempDir::new("tickgen-client-pki")?;
    let url = format!("opc.tcp://{address}/AdaptiveEnergySystem/TickGen");

    let config = ServerConfig {
        host: "127.0.0.1".to_owned(),
        port: address.port(),
        discovery_url: Some(url.clone()),
        pki_dir: server_pki.path().to_owned(),
        tick_interval: Duration::from_millis(900),
        ..Default::default()
    };
    let server = TickGenServer::new(config)?;
    let server_handle = server.handle();
    let server_task = tokio::spawn(server.run_with_listener(listener));

    let mut client = ClientBuilder::new()
        .application_name("TickGen Integration Test")
        .application_uri("urn:AdaptiveEnergySystem:IntegrationTest")
        .product_uri("urn:AdaptiveEnergySystem:IntegrationTest")
        .trust_server_certs(true)
        .create_sample_keypair(true)
        .pki_dir(client_pki.path())
        .session_retry_limit(3)
        .client()
        .map_err(|errors| std::io::Error::other(format!("{errors:?}")))?;

    let (session, event_loop) = {
        let mut connected = None;
        for attempt in 0..50 {
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
        connected.expect("connection attempts exhausted")
    };
    let event_task = event_loop.spawn();
    assert!(session.wait_for_connection().await);
    let ns = session.get_namespace_index(NAMESPACE_URI).await?;
    let object = NodeId::new(ns, "TickGenerator");

    let (tick_tx, mut tick_rx) = mpsc::unbounded_channel();
    let subscription_id = session
        .create_subscription(
            Duration::from_millis(25),
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
    session
        .create_monitored_items(
            subscription_id,
            TimestampsToReturn::Both,
            vec![NodeId::new(ns, "TickGenerator.CurrentTick").into()],
        )
        .await?;
    assert_eq!(next_tick(&mut tick_rx).await?, 0);

    let result = call_start(&session, &object, ns, 2).await?;
    assert!(result.status_code.is_good());

    assert_eq!(next_tick(&mut tick_rx).await?, 1);
    assert_eq!(next_tick(&mut tick_rx).await?, 2);

    let elapsed_node: ReadValueId = NodeId::new(ns, "TickGenerator.TimeSinceLastTick").into();
    let first = read_elapsed(&session, &elapsed_node).await?;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let second = read_elapsed(&session, &elapsed_node).await?;
    assert!(first > 0.0);
    assert!(second > first);

    // The run completes one tick interval after its final tick is emitted.
    tokio::time::sleep(Duration::from_millis(900)).await;
    let running_node: ReadValueId = NodeId::new(ns, "TickGenerator.Running").into();
    let running = session
        .read(&[running_node], TimestampsToReturn::Neither, 0.0)
        .await?;
    assert_eq!(running[0].value, Some(Variant::Boolean(false)));

    session.disconnect().await?;
    event_task.await?;
    server_handle.cancel();
    server_task.await??;
    Ok(())
}

async fn call_start(
    session: &Session,
    object: &NodeId,
    ns: u16,
    ticks: u64,
) -> Result<CallMethodResult, Box<dyn std::error::Error>> {
    let result = session
        .call_one((
            object.clone(),
            NodeId::new(ns, "TickGenerator.Start"),
            Some(vec![Variant::UInt64(ticks)]),
        ))
        .await?;
    if result.status_code != StatusCode::Good {
        return Err(format!("Start failed: {}", result.status_code).into());
    }
    Ok(result)
}

async fn next_tick(
    receiver: &mut mpsc::UnboundedReceiver<u64>,
) -> Result<u64, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await?
        .ok_or_else(|| "Tick channel closed".into())
}

async fn read_elapsed(
    session: &Session,
    node: &ReadValueId,
) -> Result<f64, Box<dyn std::error::Error>> {
    let values = session
        .read(std::slice::from_ref(node), TimestampsToReturn::Neither, 0.0)
        .await?;
    match values[0].value {
        Some(Variant::Double(milliseconds)) => Ok(milliseconds),
        ref value => Err(format!("Unexpected TimeSinceLastTick value: {value:?}").into()),
    }
}
