use opcua::{
    server::{
        address_space::{AddressSpace, MethodBuilder},
        node_manager::memory::SimpleNodeManager,
    },
    types::{Argument, DataTypeId, NodeId, StatusCode, Variant},
};

use crate::StateError;

use super::shared::SharedState;

/// Register the TickGenerator control methods and their callbacks.
pub(super) fn add_method_callbacks(
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
    address_space: &mut AddressSpace,
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
