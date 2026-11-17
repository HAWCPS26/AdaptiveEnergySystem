use std::sync::Arc;

use opcua::{
    server::{node_manager::memory::SimpleNodeManager, SubscriptionCache},
    types::{DataValue, DateTime as OpcUaDateTime, NodeId, Variant},
};
use tokio::task::JoinHandle;
use tracing::error;

use crate::TickSnapshot;

use super::{address_space::NodeIds, shared::SharedState};

/// Publish state snapshots to OPC UA subscriptions.
pub(super) fn spawn_publisher(
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

/// Advance ticks at the configured interval.
pub(super) fn spawn_ticker(shared: SharedState) -> JoinHandle<()> {
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
