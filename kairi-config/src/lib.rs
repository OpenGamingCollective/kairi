use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub discord: DiscordConfig,
    pub github: GithubConfig,
    #[serde(default)]
    pub typesafe: TypesafeConfig,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
/// TypeSafe account config used for Jev
pub struct TypesafeConfig {
    pub token: String,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct DiscordConfig {
    pub token: String,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct GithubConfig {
    pub app_id: u64,
    pub pem_file: String,
}

impl Config {
    pub fn build(file_content: &str) -> Result<Self> {
        toml::from_str(file_content).context("Failed to parse config")
    }
}
