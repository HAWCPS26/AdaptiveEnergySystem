use std::{env, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use opcua::{
    crypto::SecurityPolicy,
    server::{
        address_space::{MethodBuilder, ObjectBuilder, VariableBuilder},
        diagnostics::NamespaceMetadata,
        node_manager::memory::{simple_node_manager, SimpleNodeManager},
        Server, ServerBuilder, ServerHandle, SubscriptionCache,
    },
    types::{
        Argument, BuildInfo, DataTypeId, DataValue, DateTime as OpcUaDateTime, MessageSecurityMode,
        NodeId, StatusCode, Variant,
    },
};
use parking_lot::Mutex;
use thiserror::Error;
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};
use tracing::{error, warn};

use crate::{StateError, TickSnapshot, TickState, SIMULATED_TICK_MINUTES};

pub const NAMESPACE_URI: &str = "urn:AdaptiveEnergySystem";
const APPLICATION_URI: &str = "urn:AdaptiveEnergySystem:TickGen";
const DEFAULT_ENDPOINT_PATH: &str = "/AdaptiveEnergySystem/TickGen";

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub endpoint_path: String,
    pub discovery_url: Option<String>,
    pub pki_dir: PathBuf,
    pub tick_interval: Duration,
    pub start_time: DateTime<Utc>,
    pub start_ticks: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 4840,
            endpoint_path: DEFAULT_ENDPOINT_PATH.to_owned(),
            discovery_url: None,
            pki_dir: PathBuf::from("./pki"),
            tick_interval: Duration::from_secs(10),
            start_time: Utc::now(),
            start_ticks: 0,
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, ServerError> {
        let mut config = Self::default();
        config.host = env::var("TICKGEN_HOST").unwrap_or(config.host);
        config.port = parse_env("TICKGEN_PORT", config.port)?;
        config.endpoint_path = env::var("TICKGEN_ENDPOINT_PATH").unwrap_or(config.endpoint_path);
        config.discovery_url = env::var("TICKGEN_DISCOVERY_URL").ok();
        config.pki_dir = env::var_os("TICKGEN_PKI_DIR")
            .map(PathBuf::from)
            .unwrap_or(config.pki_dir);
        config.start_ticks = parse_env("TICKGEN_START_TICKS", config.start_ticks)?;

        let interval_seconds = parse_env(
            "TICKGEN_TICK_INTERVAL_SECONDS",
            config.tick_interval.as_secs_f64(),
        )?;
        config.tick_interval = duration_from_seconds(interval_seconds)?;

        if let Ok(start_time) = env::var("TICKGEN_START_TIME") {
            config.start_time = DateTime::parse_from_rfc3339(&start_time)
                .map_err(|error| {
                    ServerError::Configuration(format!(
                        "TICKGEN_START_TIME must be RFC 3339: {error}"
                    ))
                })?
                .with_timezone(&Utc);
        }

        if !config.endpoint_path.starts_with('/') {
            return Err(ServerError::Configuration(
                "TICKGEN_ENDPOINT_PATH must start with '/'".to_owned(),
            ));
        }
        Ok(config)
    }

    pub fn endpoint_url(&self) -> String {
        self.discovery_url.clone().unwrap_or_else(|| {
            format!(
                "opc.tcp://{}:{}{}",
                self.host, self.port, self.endpoint_path
            )
        })
    }
}

fn parse_env<T>(name: &str, default: T) -> Result<T, ServerError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Ok(value) => value.parse::<T>().map_err(|error| {
            ServerError::Configuration(format!("Invalid {name} value {value:?}: {error}"))
        }),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(ServerError::Configuration(format!(
            "Could not read {name}: {error}"
        ))),
    }
}

fn duration_from_seconds(seconds: f64) -> Result<Duration, ServerError> {
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(ServerError::Configuration(
            "Tick interval must be finite and greater than zero".to_owned(),
        ));
    }
    Duration::try_from_secs_f64(seconds)
        .map_err(|error| ServerError::Configuration(format!("Invalid tick interval: {error}")))
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("OPC UA server error: {0}")]
    OpcUa(String),
    #[error(transparent)]
    State(#[from] StateError),
}

#[derive(Clone)]
struct SharedState {
    state: Arc<Mutex<TickState>>,
    changes: watch::Sender<TickSnapshot>,
}

impl SharedState {
    fn new(state: TickState) -> Self {
        let state = Arc::new(Mutex::new(state));
        let snapshot = state.lock().snapshot();
        let (changes, _) = watch::channel(snapshot);
        Self { state, changes }
    }

    fn snapshot(&self) -> TickSnapshot {
        self.state.lock().snapshot()
    }

    fn subscribe(&self) -> watch::Receiver<TickSnapshot> {
        self.changes.subscribe()
    }

    fn start(&self, ticks: u64) -> Result<(), StateError> {
        self.mutate(|state| state.start(ticks))
    }

    fn pause(&self) -> Result<(), StateError> {
        self.mutate(|state| state.pause())
    }

    fn resume(&self) -> Result<(), StateError> {
        self.mutate(|state| state.resume())
    }

    fn reset(&self, start_time: DateTime<Utc>) {
        self.mutate(|state| {
            state.reset(start_time);
            Ok(())
        })
        .expect("reset cannot fail");
    }

    fn set_tick_interval(&self, seconds: f64) -> Result<(), StateError> {
        let interval =
            duration_from_seconds(seconds).map_err(|_| StateError::InvalidTickInterval)?;
        self.mutate(|state| state.set_tick_interval(interval))
    }

    fn advance(&self) {
        self.mutate(|state| {
            state.advance();
            Ok(())
        })
        .expect("advancing cannot fail");
    }

    fn mutate(
        &self,
        operation: impl FnOnce(&mut TickState) -> Result<(), StateError>,
    ) -> Result<(), StateError> {
        let snapshot = {
            let mut state = self.state.lock();
            operation(&mut state)?;
            state.snapshot()
        };
        self.changes.send_replace(snapshot);
        Ok(())
    }

    fn time_since_last_tick_ms(&self) -> f64 {
        self.snapshot()
            .time_since_last_tick
            .map_or(0.0, |duration| duration.as_secs_f64() * 1_000.0)
    }
}

#[derive(Clone)]
struct NodeIds {
    object: NodeId,
    current_tick: NodeId,
    simulation_time: NodeId,
    running: NodeId,
    remaining_ticks: NodeId,
    start_time: NodeId,
    tick_interval: NodeId,
    time_since_last_tick: NodeId,
}

impl NodeIds {
    fn new(ns: u16) -> Self {
        let node = |name: &str| NodeId::new(ns, format!("TickGenerator.{name}"));
        Self {
            object: NodeId::new(ns, "TickGenerator"),
            current_tick: node("CurrentTick"),
            simulation_time: node("SimulationTime"),
            running: node("Running"),
            remaining_ticks: node("RemainingTicks"),
            start_time: node("StartTime"),
            tick_interval: node("TickIntervalSeconds"),
            time_since_last_tick: node("TimeSinceLastTick"),
        }
    }
}

pub struct TickGenServer {
    server: Server,
    handle: ServerHandle,
    shared: SharedState,
    tasks: Vec<JoinHandle<()>>,
}

impl TickGenServer {
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

    pub fn handle(&self) -> ServerHandle {
        self.handle.clone()
    }

    pub fn start(&self, ticks: u64) -> Result<(), StateError> {
        self.shared.start(ticks)
    }

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

fn add_address_space(node_manager: &SimpleNodeManager, nodes: &NodeIds, snapshot: &TickSnapshot) {
    let mut address_space = node_manager.address_space().write();
    ObjectBuilder::new(&nodes.object, "TickGenerator", "TickGenerator")
        .organized_by(NodeId::objects_folder_id())
        .insert(&mut *address_space);

    let variables = [
        (
            &nodes.current_tick,
            "CurrentTick",
            "Latest emitted tick; zero means no tick has been emitted",
            Variant::from(snapshot.tick),
            DataTypeId::UInt64,
        ),
        (
            &nodes.simulation_time,
            "SimulationTime",
            "Simulation time at the start of the current tick",
            Variant::from(OpcUaDateTime::from(snapshot.simulation_time)),
            DataTypeId::DateTime,
        ),
        (
            &nodes.running,
            "Running",
            "Whether ticks are currently being generated",
            Variant::from(snapshot.running),
            DataTypeId::Boolean,
        ),
        (
            &nodes.remaining_ticks,
            "RemainingTicks",
            "Number of ticks left in the current run, including the active tick",
            Variant::from(snapshot.remaining_ticks),
            DataTypeId::UInt64,
        ),
        (
            &nodes.start_time,
            "StartTime",
            "Simulation time corresponding to tick zero",
            Variant::from(OpcUaDateTime::from(snapshot.start_time)),
            DataTypeId::DateTime,
        ),
        (
            &nodes.tick_interval,
            "TickIntervalSeconds",
            "Wall-clock duration between ticks",
            Variant::from(snapshot.tick_interval.as_secs_f64()),
            DataTypeId::Double,
        ),
        (
            &nodes.time_since_last_tick,
            "TimeSinceLastTick",
            "Wall-clock milliseconds since the current or last tick started; zero before the first tick",
            Variant::from(
                snapshot
                    .time_since_last_tick
                    .map_or(0.0, |duration| duration.as_secs_f64() * 1_000.0),
            ),
            DataTypeId::Duration,
        ),
    ];

    for (id, name, description, value, data_type) in variables {
        VariableBuilder::new(id, name, name)
            .description(description)
            .component_of(nodes.object.clone())
            .data_type(data_type)
            .value(value)
            .insert(&mut *address_space);
    }

    VariableBuilder::new(
        &NodeId::new(nodes.object.namespace, "TickGenerator.SimulatedTickMinutes"),
        "SimulatedTickMinutes",
        "SimulatedTickMinutes",
    )
    .description("Simulated duration of one complete tick")
    .component_of(nodes.object.clone())
    .data_type(DataTypeId::UInt32)
    .value(SIMULATED_TICK_MINUTES as u32)
    .insert(&mut *address_space);
}

fn add_method_callbacks(
    node_manager: &SimpleNodeManager,
    ns: u16,
    object: &NodeId,
    shared: SharedState,
) {
    let mut address_space = node_manager.address_space().write();
    let start = add_method(
        &mut address_space,
        ns,
        object,
        "Start",
        &[("TicksToRun", DataTypeId::UInt64).into()],
    );
    let pause = add_method(&mut address_space, ns, object, "Pause", &[]);
    let resume = add_method(&mut address_space, ns, object, "Resume", &[]);
    let reset = add_method(
        &mut address_space,
        ns,
        object,
        "Reset",
        &[("StartTime", DataTypeId::DateTime).into()],
    );
    let set_interval = add_method(
        &mut address_space,
        ns,
        object,
        "SetTickInterval",
        &[("Seconds", DataTypeId::Double).into()],
    );
    drop(address_space);

    let callback_state = shared.clone();
    node_manager
        .inner()
        .add_method_callback(start, move |arguments| {
            match arguments {
                [Variant::UInt64(ticks)] => callback_state.start(*ticks),
                _ => return Err(StatusCode::BadInvalidArgument),
            }
            .map(|()| Vec::new())
            .map_err(state_status)
        });

    let callback_state = shared.clone();
    node_manager
        .inner()
        .add_method_callback(pause, move |arguments| {
            if !arguments.is_empty() {
                return Err(StatusCode::BadInvalidArgument);
            }
            callback_state
                .pause()
                .map(|()| Vec::new())
                .map_err(state_status)
        });

    let callback_state = shared.clone();
    node_manager
        .inner()
        .add_method_callback(resume, move |arguments| {
            if !arguments.is_empty() {
                return Err(StatusCode::BadInvalidArgument);
            }
            callback_state
                .resume()
                .map(|()| Vec::new())
                .map_err(state_status)
        });

    let callback_state = shared.clone();
    node_manager
        .inner()
        .add_method_callback(reset, move |arguments| match arguments {
            [Variant::DateTime(start_time)] => {
                callback_state.reset(start_time.as_chrono());
                Ok(Vec::new())
            }
            _ => Err(StatusCode::BadInvalidArgument),
        });

    node_manager
        .inner()
        .add_method_callback(set_interval, move |arguments| {
            match arguments {
                [Variant::Double(seconds)] => shared.set_tick_interval(*seconds),
                _ => return Err(StatusCode::BadInvalidArgument),
            }
            .map(|()| Vec::new())
            .map_err(state_status)
        });
}

fn add_method(
    address_space: &mut opcua::server::address_space::AddressSpace,
    ns: u16,
    object: &NodeId,
    name: &str,
    arguments: &[Argument],
) -> NodeId {
    let method_id = NodeId::new(ns, format!("TickGenerator.{name}"));
    let inputs_id = NodeId::new(ns, format!("TickGenerator.{name}.InputArguments"));
    MethodBuilder::new(&method_id, name, name)
        .component_of(object.clone())
        .input_args(address_space, &inputs_id, arguments)
        .insert(address_space);
    method_id
}

fn state_status(error: StateError) -> StatusCode {
    match error {
        StateError::InvalidTickCount | StateError::InvalidTickInterval => {
            StatusCode::BadInvalidArgument
        }
        StateError::TickOverflow => StatusCode::BadOutOfRange,
        StateError::AlreadyRunning
        | StateError::NotRunning
        | StateError::NoActiveRun
        | StateError::RunAlreadyConfigured => StatusCode::BadInvalidState,
    }
}

fn spawn_publisher(
    shared: SharedState,
    node_manager: Arc<SimpleNodeManager>,
    subscriptions: Arc<SubscriptionCache>,
    nodes: NodeIds,
) -> JoinHandle<()> {
    let mut changes = shared.subscribe();
    tokio::spawn(async move {
        loop {
            if changes.changed().await.is_err() {
                break;
            }
            let snapshot = changes.borrow_and_update().clone();
            publish_snapshot(&node_manager, &subscriptions, &nodes, &snapshot);
        }
    })
}

fn publish_snapshot(
    node_manager: &SimpleNodeManager,
    subscriptions: &SubscriptionCache,
    nodes: &NodeIds,
    snapshot: &TickSnapshot,
) {
    let now = OpcUaDateTime::now();
    let values: [(&NodeId, Variant); 6] = [
        (&nodes.current_tick, snapshot.tick.into()),
        (
            &nodes.simulation_time,
            OpcUaDateTime::from(snapshot.simulation_time).into(),
        ),
        (&nodes.running, snapshot.running.into()),
        (&nodes.remaining_ticks, snapshot.remaining_ticks.into()),
        (
            &nodes.start_time,
            OpcUaDateTime::from(snapshot.start_time).into(),
        ),
        (
            &nodes.tick_interval,
            snapshot.tick_interval.as_secs_f64().into(),
        ),
    ];

    let values = values
        .into_iter()
        .map(|(node, value)| (node, None, DataValue::new_at(value, now)));
    if let Err(error) = node_manager.set_values(subscriptions, values) {
        error!("Failed to publish TickGen state: {error:?}");
    }
}

fn spawn_ticker(shared: SharedState) -> JoinHandle<()> {
    let mut changes = shared.subscribe();
    tokio::spawn(async move {
        loop {
            let snapshot = changes.borrow_and_update().clone();
            if snapshot.running {
                tokio::select! {
                    () = tokio::time::sleep(snapshot.tick_interval) => shared.advance(),
                    result = changes.changed() => {
                        if result.is_err() {
                            break;
                        }
                    }
                }
            } else if changes.changed().await.is_err() {
                break;
            }
        }
    })
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
