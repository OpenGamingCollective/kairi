use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use voting::{Abstentions, BlockWeight, Exhausted, LastTwo};

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
/// Rules for counting votes on mailing list threads
pub struct VotingConfigFile {
    /// Path to the infra inventory JSON (members and groups). Votes can't be counted without it
    pub roster_path: PathBuf,
    /// Fraction of an organisation's members who must vote for it to count (0 to 1)
    pub internal_quorum: f64,
    /// Fraction of the entities that must count for the vote to be valid (0 to 1)
    pub coalition_quorum: f64,
    /// How long a vote stays open if its first mail doesn't say
    pub window_hours: u32,
    /// Which members the unaffiliated block weighs the square root of, `voters` or `roster`
    pub unaffiliated_weight: BlockWeight,
    /// How abstentions are treated: `counted`, `quorum_only` or `ignored`
    pub abstentions: Abstentions,
    /// What happens to ballots whose options were all eliminated: `counted` or `dropped`
    pub exhausted_ballots: Exhausted,
    /// What to do when two options are left and neither has a majority: `runoff` or `final`
    pub last_two_options: LastTwo,
    /// Lets `/voteconfig` change the rules until the next restart. Meant for test bots
    pub rule_changes: bool,
}

impl Default for VotingConfigFile {
    fn default() -> Self {
        Self {
            roster_path: PathBuf::new(),
            internal_quorum: 0.5,
            coalition_quorum: 0.5,
            window_hours: 24,
            unaffiliated_weight: BlockWeight::default(),
            abstentions: Abstentions::default(),
            exhausted_ballots: Exhausted::default(),
            last_two_options: LastTwo::default(),
            rule_changes: false,
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
/// Rules for counting votes on mailing list threads
pub struct VotingConfig {
    /// Read again on every count. `None` if it isn't set
    pub roster_path: Option<PathBuf>,
    pub internal_quorum: f64,
    pub coalition_quorum: f64,
    pub window_hours: u32,
    pub unaffiliated_weight: BlockWeight,
    pub abstentions: Abstentions,
    pub exhausted_ballots: Exhausted,
    pub last_two_options: LastTwo,
    /// Lets `/voteconfig` change the rules until the bot restarts
    pub rule_changes: bool,
}

impl VotingConfig {
    pub fn check(&self) -> std::result::Result<(), String> {
        for (name, quorum) in [
            ("internal_quorum", self.internal_quorum),
            ("coalition_quorum", self.coalition_quorum),
        ] {
            if !(0.0..=1.0).contains(&quorum) {
                return Err(format!("{name} must be between 0 and 1, got {quorum}"));
            }
        }
        if self.window_hours == 0 {
            return Err("window_hours must be above 0".to_string());
        }
        Ok(())
    }
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
        let voting = VotingConfig {
            roster_path: (!voting.roster_path.is_empty()).then_some(voting.roster_path),
            internal_quorum: voting.internal_quorum,
            coalition_quorum: voting.coalition_quorum,
            window_hours: voting.window_hours,
            unaffiliated_weight: voting.unaffiliated_weight,
            abstentions: voting.abstentions,
            exhausted_ballots: voting.exhausted_ballots,
            last_two_options: voting.last_two_options,
            rule_changes: voting.rule_changes,
        };
        voting.check().map_err(|error| anyhow!("voting.{error}"))?;

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
        assert_eq!(voting.abstentions, Abstentions::Counted);
        assert!(!voting.rule_changes);
    }

    #[test]
    fn voting_rules_are_read() {
        let config = format!(
            "{MINIMAL}[voting]\nunaffiliated_weight = \"roster\"\nabstentions = \"quorum_only\"\n\
             exhausted_ballots = \"dropped\"\nlast_two_options = \"final\"\nrule_changes = true\n"
        );
        let voting = Config::build(&config).unwrap().voting;
        assert_eq!(voting.unaffiliated_weight, BlockWeight::Roster);
        assert_eq!(voting.abstentions, Abstentions::QuorumOnly);
        assert_eq!(voting.exhausted_ballots, Exhausted::Dropped);
        assert_eq!(voting.last_two_options, LastTwo::Final);
        assert!(voting.rule_changes);

        let config = format!("{MINIMAL}[voting]\nabstentions = \"sometimes\"\n");
        assert!(Config::build(&config).is_err());
    }

    #[test]
    fn quorums_are_shares() {
        let config = format!("{MINIMAL}[voting]\ncoalition_quorum = 50\n");
        assert!(Config::build(&config).is_err());
        let config = format!("{MINIMAL}[voting]\nwindow_hours = 0\n");
        assert!(Config::build(&config).is_err());
    }
}
