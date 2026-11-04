use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub discord: DiscordConfig,
    pub github: GithubConfig,
    pub clef: JevConfig,
    pub cloudflare: CloudflareConfig
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct JevConfig {
    pub model: String,
    pub provider: JevProvider,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub enum JevProvider {
    #[default]
    Cloudflare,
    TypesafeAi,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
/// Cloudflare account config used for clef
pub struct CloudflareConfig {
    pub account_id: String,
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
