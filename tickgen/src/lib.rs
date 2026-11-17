//! OPC UA tick generator for the adaptive energy system simulation.

mod server;
mod state;

pub use server::{ServerConfig, ServerError, TickGenServer, NAMESPACE_URI};
pub use state::{StateError, TickSnapshot, TickState, SIMULATED_TICK_MINUTES};
