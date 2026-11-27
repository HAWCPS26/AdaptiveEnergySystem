use std::sync::Arc;

use crate::{DeferrableLoad, LoadConfig, LoadError, LoadSnapshot, SharedLoad, NAMESPACE_URI};
use chrono::Utc;
use opcua::{
    crypto::SecurityPolicy,
    server::{
        address_space::{AddressSpace, MethodBuilder, ObjectBuilder, VariableBuilder},
        diagnostics::NamespaceMetadata,
        node_manager::memory::{simple_node_manager, SimpleNodeManager},
        Server, ServerBuilder, ServerHandle, SubscriptionCache,
    },
    types::{
        Argument, BuildInfo, DataTypeId, DataValue, DateTime as OpcUaDateTime, MessageSecurityMode,
        NodeId, NumericRange, StatusCode, Variant,
    },
};
use tokio::{net::TcpListener, task::JoinHandle};
use tracing::{info, warn};

use crate::client;

const APPLICATION_URI: &str = "urn:AdaptiveEnergySystem:DeferrableLoad";

#[derive(Clone)]
struct LoadNodeIds {
    object: NodeId,
    state: NodeId,
    phase: NodeId,
    deadline_set: NodeId,
    deadline: NodeId,
    latest_start: NodeId,
    deadline_met: NodeId,
    cycle_duration: NodeId,
    cycle_elapsed: NodeId,
    cycle_remaining: NodeId,
    progress: NodeId,
    current_power: NodeId,
    energy_delivered: NodeId,
    cycle_energy: NodeId,
    simulation_time: NodeId,
    tickgen_connected: NodeId,
}

impl LoadNodeIds {
    fn new(ns: u16) -> Self {
        let node = |name: &str| NodeId::new(ns, format!("DeferrableLoad.{name}"));
        Self {
            object: NodeId::new(ns, "DeferrableLoad"),
            state: node("State"),
            phase: node("Phase"),
            deadline_set: node("DeadlineSet"),
            deadline: node("Deadline"),
            latest_start: node("LatestStart"),
            deadline_met: node("DeadlineMet"),
            cycle_duration: node("CycleDurationMinutes"),
            cycle_elapsed: node("CycleElapsedMinutes"),
            cycle_remaining: node("CycleRemainingMinutes"),
            progress: node("ProgressPercent"),
            current_power: node("CurrentPowerKw"),
            energy_delivered: node("EnergyDeliveredKwh"),
            cycle_energy: node("CycleEnergyKwh"),
            simulation_time: node("SimulationTime"),
            tickgen_connected: node("TickGenConnected"),
        }
    }
}

pub struct DeferrableLoadServer {
    server: Server,
    handle: ServerHandle,
    shared: SharedLoad,
    tasks: Vec<JoinHandle<()>>,
}

impl DeferrableLoadServer {
    /// Build the washing-machine server and its OPC UA address space.
    pub fn new(config: LoadConfig) -> Result<Self, ServerError> {
        let shared = SharedLoad::new(DeferrableLoad::new());
        let endpoint_url = config.endpoint_url();
        let user_tokens = ["ANONYMOUS"];

        let (server, handle) = ServerBuilder::new()
            .application_name("Adaptive Energy System Washing Machine")
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
                product_name: "DeferrableLoad".into(),
                software_version: env!("CARGO_PKG_VERSION").into(),
                build_number: "1".into(),
                build_date: OpcUaDateTime::now(),
            })
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: NAMESPACE_URI.to_owned(),
                    ..Default::default()
                },
                "deferrable_load",
            ))
            .build()
            .map_err(|error| ServerError::OpcUa(error.to_string()))?;

        let node_manager = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .ok_or_else(|| ServerError::OpcUa("load node manager is missing".to_owned()))?;
        let ns = handle
            .get_namespace_index(NAMESPACE_URI)
            .ok_or_else(|| ServerError::OpcUa("load namespace is missing".to_owned()))?;
        let nodes = LoadNodeIds::new(ns);

        add_address_space(&node_manager, &nodes, &shared.snapshot());
        add_method_callbacks(&node_manager, ns, &nodes.object, shared.clone());

        let publisher = spawn_publisher(
            shared.clone(),
            node_manager,
            handle.subscriptions().clone(),
            nodes,
        );

        Ok(Self {
            server,
            handle,
            shared,
            tasks: vec![publisher],
        })
    }

    /// Follow TickGen and advance the appliance on each tick.
    pub fn spawn_sync(&mut self, config: LoadConfig) {
        let task = client::spawn_sync(config, self.shared.clone());
        self.tasks.push(task);
    }

    /// Return a handle for shutting down the server.
    pub fn handle(&self) -> ServerHandle {
        self.handle.clone()
    }

    /// Run the OPC UA server until it is cancelled.
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

    /// Run the OPC UA server on an existing TCP listener.
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

fn add_address_space(
    node_manager: &SimpleNodeManager,
    nodes: &LoadNodeIds,
    snapshot: &LoadSnapshot,
) {
    let mut address_space = node_manager.address_space().write();
    ObjectBuilder::new(&nodes.object, "DeferrableLoad", "DeferrableLoad")
        .description("Externally controlled deferrable washing machine")
        .organized_by(NodeId::objects_folder_id())
        .insert(&mut *address_space);

    let fallback_time = snapshot.simulation_time.unwrap_or_else(Utc::now);
    let deadline = snapshot.deadline.unwrap_or(fallback_time);
    let latest_start = snapshot.latest_start.unwrap_or(fallback_time);

    let variables = [
        (
            &nodes.state,
            "State",
            "Idle, Running, Paused, Done or Stopped",
            Variant::from(snapshot.state.as_str()),
            DataTypeId::String,
        ),
        (
            &nodes.phase,
            "Phase",
            "Current wash phase",
            Variant::from(snapshot.phase),
            DataTypeId::String,
        ),
        (
            &nodes.deadline_set,
            "DeadlineSet",
            "Whether a deadline has been configured",
            Variant::from(snapshot.deadline.is_some()),
            DataTypeId::Boolean,
        ),
        (
            &nodes.deadline,
            "Deadline",
            "Simulation time by which the cycle must finish",
            Variant::from(OpcUaDateTime::from(deadline)),
            DataTypeId::DateTime,
        ),
        (
            &nodes.latest_start,
            "LatestStart",
            "Latest start time that still finishes before Deadline, assuming no pauses",
            Variant::from(OpcUaDateTime::from(latest_start)),
            DataTypeId::DateTime,
        ),
        (
            &nodes.deadline_met,
            "DeadlineMet",
            "Whether the last completed cycle finished before Deadline",
            Variant::from(snapshot.deadline_met),
            DataTypeId::Boolean,
        ),
        (
            &nodes.cycle_duration,
            "CycleDurationMinutes",
            "Total duration of one wash cycle in simulated minutes",
            Variant::from(snapshot.cycle_duration_minutes),
            DataTypeId::UInt32,
        ),
        (
            &nodes.cycle_elapsed,
            "CycleElapsedMinutes",
            "Simulated minutes completed in the current or last cycle",
            Variant::from(snapshot.cycle_elapsed_minutes),
            DataTypeId::Double,
        ),
        (
            &nodes.cycle_remaining,
            "CycleRemainingMinutes",
            "Simulated minutes left in the current or last cycle",
            Variant::from(snapshot.cycle_remaining_minutes),
            DataTypeId::Double,
        ),
        (
            &nodes.progress,
            "ProgressPercent",
            "Cycle progress from 0 to 100",
            Variant::from(snapshot.progress_percent),
            DataTypeId::Double,
        ),
        (
            &nodes.current_power,
            "CurrentPowerKw",
            "Average power during the most recent tick",
            Variant::from(snapshot.current_power_kw),
            DataTypeId::Double,
        ),
        (
            &nodes.energy_delivered,
            "EnergyDeliveredKwh",
            "Energy consumed by the current or last cycle",
            Variant::from(snapshot.energy_delivered_kwh),
            DataTypeId::Double,
        ),
        (
            &nodes.cycle_energy,
            "CycleEnergyKwh",
            "Nominal energy of one complete wash cycle",
            Variant::from(snapshot.cycle_energy_kwh),
            DataTypeId::Double,
        ),
        (
            &nodes.simulation_time,
            "SimulationTime",
            "Most recent simulation time processed by the appliance",
            Variant::from(OpcUaDateTime::from(fallback_time)),
            DataTypeId::DateTime,
        ),
        (
            &nodes.tickgen_connected,
            "TickGenConnected",
            "Whether the appliance is connected to TickGen",
            Variant::from(snapshot.tickgen_connected),
            DataTypeId::Boolean,
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
}

fn add_method_callbacks(
    node_manager: &SimpleNodeManager,
    ns: u16,
    object: &NodeId,
    shared: SharedLoad,
) {
    let mut address_space = node_manager.address_space().write();
    let set_deadline = add_method(
        &mut address_space,
        ns,
        object,
        "SetDeadline",
        &[("Deadline", DataTypeId::DateTime).into()],
    );
    let start = add_method(&mut address_space, ns, object, "Start", &[]);
    let pause = add_method(&mut address_space, ns, object, "Pause", &[]);
    let resume = add_method(&mut address_space, ns, object, "Resume", &[]);
    let stop = add_method(&mut address_space, ns, object, "Stop", &[]);
    drop(address_space);

    let callback_state = shared.clone();
    node_manager
        .inner()
        .add_method_callback(set_deadline, move |arguments| match arguments {
            [Variant::DateTime(deadline)] => {
                let deadline = deadline.as_chrono();
                callback_state
                    .set_deadline(deadline)
                    .map(|()| {
                        info!(
                            deadline = %deadline.to_rfc3339(),
                            "washing machine deadline updated"
                        );
                        Vec::new()
                    })
                    .map_err(load_status)
            }
            _ => Err(StatusCode::BadInvalidArgument),
        });

    let callback_state = shared.clone();
    node_manager
        .inner()
        .add_method_callback(start, move |arguments| {
            if !arguments.is_empty() {
                return Err(StatusCode::BadInvalidArgument);
            }
            callback_state
                .start()
                .map(|()| {
                    info!("washing machine cycle started");
                    Vec::new()
                })
                .map_err(load_status)
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
                .map(|()| {
                    info!("washing machine cycle paused");
                    Vec::new()
                })
                .map_err(load_status)
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
                .map(|()| {
                    info!("washing machine cycle resumed");
                    Vec::new()
                })
                .map_err(load_status)
        });

    node_manager
        .inner()
        .add_method_callback(stop, move |arguments| {
            if !arguments.is_empty() {
                return Err(StatusCode::BadInvalidArgument);
            }
            shared
                .stop()
                .map(|()| {
                    info!("washing machine cycle stopped");
                    Vec::new()
                })
                .map_err(load_status)
        });
}

fn add_method(
    address_space: &mut AddressSpace,
    ns: u16,
    object: &NodeId,
    name: &str,
    arguments: &[Argument],
) -> NodeId {
    let method_id = NodeId::new(ns, format!("DeferrableLoad.{name}"));
    let inputs_id = NodeId::new(ns, format!("DeferrableLoad.{name}.InputArguments"));
    MethodBuilder::new(&method_id, name, name)
        .component_of(object.clone())
        .input_args(address_space, &inputs_id, arguments)
        .insert(address_space);
    method_id
}

fn load_status(error: LoadError) -> StatusCode {
    match error {
        LoadError::AlreadyRunning
        | LoadError::NotRunning
        | LoadError::NotPaused
        | LoadError::NoDeadline => StatusCode::BadInvalidState,
    }
}

fn spawn_publisher(
    shared: SharedLoad,
    node_manager: Arc<SimpleNodeManager>,
    subscriptions: Arc<SubscriptionCache>,
    nodes: LoadNodeIds,
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
    nodes: &LoadNodeIds,
    snapshot: &LoadSnapshot,
) {
    let now = OpcUaDateTime::now();
    let fallback_time = snapshot.simulation_time.unwrap_or_else(Utc::now);
    let deadline = snapshot.deadline.unwrap_or(fallback_time);
    let latest_start = snapshot.latest_start.unwrap_or(fallback_time);

    let values: [(&NodeId, Option<&NumericRange>, DataValue); 15] = [
        (
            &nodes.state,
            None,
            DataValue::new_at(Variant::from(snapshot.state.as_str()), now),
        ),
        (
            &nodes.phase,
            None,
            DataValue::new_at(Variant::from(snapshot.phase), now),
        ),
        (
            &nodes.deadline_set,
            None,
            DataValue::new_at(Variant::from(snapshot.deadline.is_some()), now),
        ),
        (
            &nodes.deadline,
            None,
            DataValue::new_at(Variant::from(OpcUaDateTime::from(deadline)), now),
        ),
        (
            &nodes.latest_start,
            None,
            DataValue::new_at(Variant::from(OpcUaDateTime::from(latest_start)), now),
        ),
        (
            &nodes.deadline_met,
            None,
            DataValue::new_at(Variant::from(snapshot.deadline_met), now),
        ),
        (
            &nodes.cycle_duration,
            None,
            DataValue::new_at(Variant::from(snapshot.cycle_duration_minutes), now),
        ),
        (
            &nodes.cycle_elapsed,
            None,
            DataValue::new_at(Variant::from(snapshot.cycle_elapsed_minutes), now),
        ),
        (
            &nodes.cycle_remaining,
            None,
            DataValue::new_at(Variant::from(snapshot.cycle_remaining_minutes), now),
        ),
        (
            &nodes.progress,
            None,
            DataValue::new_at(Variant::from(snapshot.progress_percent), now),
        ),
        (
            &nodes.current_power,
            None,
            DataValue::new_at(Variant::from(snapshot.current_power_kw), now),
        ),
        (
            &nodes.energy_delivered,
            None,
            DataValue::new_at(Variant::from(snapshot.energy_delivered_kwh), now),
        ),
        (
            &nodes.cycle_energy,
            None,
            DataValue::new_at(Variant::from(snapshot.cycle_energy_kwh), now),
        ),
        (
            &nodes.simulation_time,
            None,
            DataValue::new_at(Variant::from(OpcUaDateTime::from(fallback_time)), now),
        ),
        (
            &nodes.tickgen_connected,
            None,
            DataValue::new_at(Variant::from(snapshot.tickgen_connected), now),
        ),
    ];

    if let Err(error) = node_manager.set_values(subscriptions, values.into_iter()) {
        warn!("Failed to publish washing-machine state: {error:?}");
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

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("OPC UA error: {0}")]
    OpcUa(String),
}
