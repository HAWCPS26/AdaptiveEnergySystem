use opcua::{
    server::{
        address_space::{ObjectBuilder, VariableBuilder},
        node_manager::memory::SimpleNodeManager,
    },
    types::{DataTypeId, DateTime as OpcUaDateTime, NodeId, Variant},
};

use crate::{TickSnapshot, SIMULATED_TICK_MINUTES};

#[derive(Clone)]
pub(super) struct NodeIds {
    pub(super) object: NodeId,
    pub(super) current_tick: NodeId,
    pub(super) simulation_time: NodeId,
    pub(super) running: NodeId,
    pub(super) remaining_ticks: NodeId,
    pub(super) start_time: NodeId,
    pub(super) tick_interval: NodeId,
    pub(super) time_since_last_tick: NodeId,
}

impl NodeIds {
    pub(super) fn new(ns: u16) -> Self {
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

/// Add the TickGenerator object and its state variables.
pub(super) fn add_address_space(
    node_manager: &SimpleNodeManager,
    nodes: &NodeIds,
    snapshot: &TickSnapshot,
) {
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
