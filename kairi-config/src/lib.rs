use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConfigFile {
    pub discord: DiscordConfigFile,
    pub github: GithubConfigFile,
    #[serde(default)]
    pub typesafe: TypesafeConfigFile,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
/// TypeSafe account config used for Jev
pub struct TypesafeConfigFile {
    pub token: String,
    pub token_path: PathBuf,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct DiscordConfigFile {
    pub token: String,
    pub token_path: PathBuf,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
pub struct GithubConfigFile {
    pub app_id: u64,
    pub pem_path: PathBuf,
}

impl ConfigFile {
    pub fn build(file_content: &str) -> Result<Self> {
        toml::from_str(file_content).context("Failed to parse config")
    }
}

/*
    Config is the struct exposed to the callers
*/
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
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
    pub pem: String,
}

impl Config {
    pub fn build(file_content: &str) -> Result<Self> {
        let config_file = ConfigFile::build(file_content)?;

        // Read directly from the path if any
        let discord_token = match config_file.discord.token_path.is_empty() {
            true => config_file.discord.token,
            false => fs::read_to_string(config_file.discord.token_path)?,
        }
        .trim()
        .to_string();

        // Read directly from the path if any
        let typesafe_token = match config_file.typesafe.token_path.is_empty() {
            true => config_file.typesafe.token,
            false => fs::read_to_string(config_file.typesafe.token_path)?,
        }
        .trim()
        .to_string();

        // Read directly from the path if any
        let github_pem = match config_file.github.pem_path.is_empty() {
            true => "".to_string(),
            false => fs::read_to_string(config_file.github.pem_path)?,
        }
        .trim()
        .to_string();

        Ok(Self {
            discord: DiscordConfig {
                token: discord_token,
            },
            github: GithubConfig {
                app_id: config_file.github.app_id,
                pem: github_pem,
            },
            typesafe: TypesafeConfig {
                token: typesafe_token,
            },
        })
    }
}
