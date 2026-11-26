use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use voting::Policy;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConfigFile {
    pub discord: DiscordConfigFile,
    pub github: GithubConfigFile,
    #[serde(default)]
    pub typesafe: TypesafeConfigFile,
    #[serde(default)]
    pub voting: VotingConfig,
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct VotingConfig {
    /// Infra inventory JSON
    pub roster_path: Option<PathBuf>,
    #[serde(flatten)]
    pub policy: Policy,
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
    #[serde(default)]
    pub voting: VotingConfig,
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

        let mut voting = config_file.voting;
        // `roster_path = ""` leaves it unset
        voting.roster_path = voting.roster_path.filter(|path| !path.is_empty());
        voting
            .policy
            .check()
            .map_err(|error| anyhow!("voting.{error}"))?;

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
            voting,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str =
        "[discord]\ntoken = \"t\"\ntoken_path = \"\"\n[github]\napp_id = 0\npem_path = \"\"\n";

    #[test]
    fn voting_defaults_to_half_quorums_without_a_roster() {
        let voting = Config::build(MINIMAL).unwrap().voting;
        assert_eq!(voting.roster_path, None);
        assert_eq!(voting.policy.internal_quorum, 0.5);
        assert_eq!(voting.policy.coalition_quorum, 0.5);
        assert_eq!(voting.policy.window_hours, 24);
    }

    #[test]
    fn voting_settings_are_read() {
        let config = format!(
            "{MINIMAL}[voting]\nroster_path = \"/run/kairi/roster.json\"\ninternal_quorum = 0.25\n"
        );
        let voting = Config::build(&config).unwrap().voting;
        assert_eq!(
            voting.roster_path,
            Some(PathBuf::from("/run/kairi/roster.json"))
        );
        assert_eq!(voting.policy.internal_quorum, 0.25);
        assert_eq!(voting.policy.coalition_quorum, 0.5);
    }

    #[test]
    fn quorums_are_shares() {
        let config = format!("{MINIMAL}[voting]\ncoalition_quorum = 50\n");
        assert!(Config::build(&config).is_err());
        let config = format!("{MINIMAL}[voting]\nwindow_hours = 0\n");
        assert!(Config::build(&config).is_err());
    }
}
