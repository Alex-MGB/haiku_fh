use serde::Deserialize;
use config::{Config as ConfigLoader, ConfigError, File, Environment};

#[derive(Deserialize)]
pub struct DeribitConfig {
    pub url: String,
    pub key: String,
    pub secret: String,
    pub channels: Vec<String>,
}

#[derive(Deserialize)]
pub struct Config {
    pub deribit: DeribitConfig,
}

impl Config {

    pub fn load() -> Result<Self, ConfigError> {
        let builder = ConfigLoader::builder()
            // The following lines need obvisoulsy to be replaced in prod
            .add_source(File::with_name("config_example").required(false))
            .add_source(Environment::with_prefix("DERIBIT").separator("__"));
        let cfg = builder.build()?;
        cfg.try_deserialize::<Self>()
    }
}
