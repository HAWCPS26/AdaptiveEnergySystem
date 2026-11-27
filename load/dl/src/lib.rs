//! Externally controlled deferrable washing machine.
//!
//! The appliance does not read prices and does not decide when to run. An
//! external energy management system sets a deadline and calls `Start`,
//! `Pause`, `Resume` or `Stop` over OPC UA.

pub mod config;
pub mod model;
pub mod profile;
pub mod server;
pub mod shared;

mod client;

pub use config::{ConfigError, LoadConfig};
pub use model::{DeferrableLoad, LoadError, LoadSnapshot, LoadState};
pub use server::{DeferrableLoadServer, ServerError};
pub use shared::SharedLoad;

/// Namespace shared by all Adaptive Energy System OPC UA servers.
pub const NAMESPACE_URI: &str = "urn:AdaptiveEnergySystem";
