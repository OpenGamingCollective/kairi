use std::{fs, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ConfigFile {
    pub discord: DiscordConfigFile,
    pub github: GithubConfigFile,
    #[serde(default)]
    pub typesafe: TypesafeConfigFile,
    #[serde(default)]
    pub voting: VotingConfigFile,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
/// How the votes of the mailing list threads are counted
pub struct VotingConfigFile {
    /// JSON of the members and groups of the infra inventory, votes can't be counted without
    pub roster_path: PathBuf,
    /// Share of an organisation's roster that has to vote for it to count, from 0 to 1
    pub internal_quorum: f64,
    /// Share of the organisations that have to count for the vote to stand, from 0 to 1
    pub coalition_quorum: f64,
    /// How long a vote lasts when its first mail doesn't say
    pub window_hours: u32,
}

impl Default for VotingConfigFile {
    fn default() -> Self {
        Self {
            roster_path: PathBuf::new(),
            internal_quorum: 0.5,
            coalition_quorum: 0.5,
            window_hours: 24,
        }
    }
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
/// How the votes of the mailing list threads are counted
pub struct VotingConfig {
    /// Read at each count, `None` when it isn't set
    pub roster_path: Option<PathBuf>,
    pub internal_quorum: f64,
    pub coalition_quorum: f64,
    pub window_hours: u32,
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

        let voting = config_file.voting;
        for (name, quorum) in [
            ("internal_quorum", voting.internal_quorum),
            ("coalition_quorum", voting.coalition_quorum),
        ] {
            ensure!(
                (0.0..=1.0).contains(&quorum),
                "voting.{name} must be between 0 and 1, got {quorum}"
            );
        }
        ensure!(
            voting.window_hours > 0,
            "voting.window_hours must be above 0"
        );

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
            voting: VotingConfig {
                roster_path: (!voting.roster_path.is_empty()).then_some(voting.roster_path),
                internal_quorum: voting.internal_quorum,
                coalition_quorum: voting.coalition_quorum,
                window_hours: voting.window_hours,
            },
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
        assert_eq!(voting.internal_quorum, 0.5);
        assert_eq!(voting.coalition_quorum, 0.5);
        assert_eq!(voting.window_hours, 24);
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
        assert_eq!(voting.internal_quorum, 0.25);
        assert_eq!(voting.coalition_quorum, 0.5);
    }

    #[test]
    fn quorums_are_shares() {
        let config = format!("{MINIMAL}[voting]\ncoalition_quorum = 50\n");
        assert!(Config::build(&config).is_err());
        let config = format!("{MINIMAL}[voting]\nwindow_hours = 0\n");
        assert!(Config::build(&config).is_err());
    }
}
