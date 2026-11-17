use std::{env, path::PathBuf, str::FromStr, time::Duration};

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::StateError;

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
    /// Load server settings from environment variables.
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

    /// Return the advertised OPC UA endpoint URL.
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

pub(super) fn duration_from_seconds(seconds: f64) -> Result<Duration, ServerError> {
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
