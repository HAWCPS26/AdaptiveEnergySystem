use std::{env, path::PathBuf, str::FromStr};

const DEFAULT_ENDPOINT_PATH: &str = "/AdaptiveEnergySystem/DeferrableLoad";
const DEFAULT_TICKGEN_URL: &str = "opc.tcp://127.0.0.1:4840/AdaptiveEnergySystem/TickGen";

#[derive(Debug, Clone)]
pub struct LoadConfig {
    pub host: String,
    pub port: u16,
    pub endpoint_path: String,
    pub discovery_url: Option<String>,
    pub pki_dir: PathBuf,
    pub client_pki_dir: PathBuf,
    pub tickgen_url: String,
}

impl Default for LoadConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: 4842,
            endpoint_path: DEFAULT_ENDPOINT_PATH.to_owned(),
            discovery_url: None,
            pki_dir: PathBuf::from("./pki"),
            client_pki_dir: PathBuf::from("./pki/client"),
            tickgen_url: DEFAULT_TICKGEN_URL.to_owned(),
        }
    }
}

impl LoadConfig {
    /// Load washing-machine settings from environment variables.
    pub fn from_env() -> Result<Self, ConfigError> {
        let mut config = Self::default();
        config.host = env::var("DL_HOST").unwrap_or(config.host);
        config.port = parse_env("DL_PORT", config.port)?;
        config.endpoint_path = env::var("DL_ENDPOINT_PATH").unwrap_or(config.endpoint_path);
        config.discovery_url = env::var("DL_DISCOVERY_URL").ok();
        config.pki_dir = env::var_os("DL_PKI_DIR")
            .map(PathBuf::from)
            .unwrap_or(config.pki_dir);
        config.client_pki_dir = env::var_os("DL_CLIENT_PKI_DIR")
            .map(PathBuf::from)
            .unwrap_or(config.client_pki_dir);
        config.tickgen_url = env::var("TICKGEN_URL").unwrap_or(config.tickgen_url);

        if !config.endpoint_path.starts_with('/') {
            return Err(ConfigError::Configuration(
                "DL_ENDPOINT_PATH must start with '/'".to_owned(),
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

fn parse_env<T>(name: &str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(name) {
        Ok(value) => value.parse::<T>().map_err(|error| {
            ConfigError::Configuration(format!("Invalid {name} value {value:?}: {error}"))
        }),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(ConfigError::Configuration(format!(
            "Could not read {name}: {error}"
        ))),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("configuration error: {0}")]
    Configuration(String),
}
