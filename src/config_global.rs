use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct DeribitConfig {
    pub url: String,
    pub key: String,
    pub secret: String,
    pub channels: Vec<String>,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub url: String,
    pub key: String,
    pub secret: String,
    pub channels: Vec<String>,
    pub log_path: String,
    pub meta_data_path: String,
}

impl Config {

    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let cfg_data = serde_json::from_str(&content)?;

        Ok(cfg_data)
    }
}
