mod address_space;
mod config;
mod methods;
mod runtime;
mod shared;

pub use config::{ServerConfig, ServerError};

use opcua::{
    crypto::SecurityPolicy,
    server::{
        diagnostics::NamespaceMetadata,
        node_manager::memory::{simple_node_manager, SimpleNodeManager},
        Server, ServerBuilder, ServerHandle,
    },
    types::{BuildInfo, DataValue, DateTime as OpcUaDateTime, MessageSecurityMode},
};
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::warn;

use crate::{StateError, TickState};

use self::{
    address_space::{add_address_space, NodeIds},
    methods::add_method_callbacks,
    runtime::{spawn_publisher, spawn_ticker},
    shared::SharedState,
};

pub const NAMESPACE_URI: &str = "urn:AdaptiveEnergySystem";
const APPLICATION_URI: &str = "urn:AdaptiveEnergySystem:TickGen";

pub struct TickGenServer {
    server: Server,
    handle: ServerHandle,
    shared: SharedState,
    tasks: Vec<JoinHandle<()>>,
}

impl TickGenServer {
    /// Build the OPC UA server and start its tick runtime.
    pub fn new(config: ServerConfig) -> Result<Self, ServerError> {
        let state = TickState::new(config.start_time, config.tick_interval)?;
        let shared = SharedState::new(state);
        let endpoint_url = config.endpoint_url();
        let user_tokens = ["ANONYMOUS"];

        let (server, handle) = ServerBuilder::new()
            .application_name("Adaptive Energy System TickGen")
            .application_uri(APPLICATION_URI)
            .product_uri(APPLICATION_URI)
            .host(config.host)
            .port(config.port)
            .discovery_urls(vec![endpoint_url])
            .create_sample_keypair(true)
            .certificate_path("own/cert.der")
            .private_key_path("private/private.pem")
            .pki_dir(config.pki_dir)
            .trust_client_certs(true)
            .diagnostics_enabled(true)
            .add_endpoint(
                "none",
                (
                    config.endpoint_path.as_str(),
                    SecurityPolicy::None,
                    MessageSecurityMode::None,
                    &user_tokens as &[&str],
                ),
            )
            .build_info(BuildInfo {
                product_uri: APPLICATION_URI.into(),
                manufacturer_name: "Adaptive Energy System".into(),
                product_name: "TickGen".into(),
                software_version: env!("CARGO_PKG_VERSION").into(),
                build_number: "1".into(),
                build_date: OpcUaDateTime::now(),
            })
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: NAMESPACE_URI.to_owned(),
                    ..Default::default()
                },
                "tickgen",
            ))
            .build()
            .map_err(ServerError::OpcUa)?;

        let node_manager = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .ok_or_else(|| ServerError::OpcUa("TickGen node manager is missing".to_owned()))?;
        let ns = handle
            .get_namespace_index(NAMESPACE_URI)
            .ok_or_else(|| ServerError::OpcUa("TickGen namespace is missing".to_owned()))?;
        let nodes = NodeIds::new(ns);

        add_address_space(&node_manager, &nodes, &shared.snapshot());
        add_method_callbacks(&node_manager, ns, &nodes.object, shared.clone());

        let read_state = shared.clone();
        let elapsed_node = nodes.time_since_last_tick.clone();
        node_manager
            .inner()
            .add_read_callback(elapsed_node, move |_, _, _| {
                Ok(DataValue::new_now(read_state.time_since_last_tick_ms()))
            });

        let publisher = spawn_publisher(
            shared.clone(),
            node_manager,
            handle.subscriptions().clone(),
            nodes,
        );
        let ticker = spawn_ticker(shared.clone());

        Ok(Self {
            server,
            handle,
            shared,
            tasks: vec![publisher, ticker],
        })
    }

    /// Return a handle for shutting down the server.
    pub fn handle(&self) -> ServerHandle {
        self.handle.clone()
    }

    /// Start a finite run and emit its first tick.
    pub fn start(&self, ticks: u64) -> Result<(), StateError> {
        self.shared.start(ticks)
    }

    /// Run the server until it is cancelled.
    pub async fn run(self) -> Result<(), ServerError> {
        let Self {
            server,
            handle,
            tasks,
            ..
        } = self;
        spawn_ctrl_c_handler(handle);
        let result = server.run().await;
        stop_tasks(&tasks);
        result.map_err(ServerError::OpcUa)
    }

    /// Run the server on an existing TCP listener.
    pub async fn run_with_listener(self, listener: TcpListener) -> Result<(), ServerError> {
        let Self {
            server,
            handle,
            tasks,
            ..
        } = self;
        spawn_ctrl_c_handler(handle);
        let result = server.run_with(listener).await;
        stop_tasks(&tasks);
        result.map_err(ServerError::OpcUa)
    }
}

fn spawn_ctrl_c_handler(handle: ServerHandle) {
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => handle.cancel(),
            Err(error) => warn!("Failed to listen for Ctrl+C: {error}"),
        }
    });
}

fn stop_tasks(tasks: &[JoinHandle<()>]) {
    for task in tasks {
        task.abort();
    }
}
