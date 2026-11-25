//! `/voteconfig` shows the voting rules. If `voting.rule_changes` allows it, it can also change
//! them until the next restart

use kairi_config::VotingConfig;
use kameo::error::SendError;
use poise::ChoiceParameter;
use voting::{Abstentions, BlockWeight, Exhausted, LastTwo};

use super::{Data, SerenityContext, SerenityError};
use crate::actors::{ChangeRules, Rules};

/// A rule offered as a command choice, named the same as in the config file
macro_rules! choice {
    ($choice:ident, $rule:ident, { $($variant:ident => $name:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, poise::ChoiceParameter)]
        pub enum $choice {
            $(#[name = $name] $variant),+
        }

        impl From<$choice> for $rule {
            fn from(choice: $choice) -> Self {
                match choice {
                    $($choice::$variant => Self::$variant),+
                }
            }
        }

        impl From<$rule> for $choice {
            fn from(rule: $rule) -> Self {
                match rule {
                    $($rule::$variant => Self::$variant),+
                }
            }
        }
    };
}

choice!(AbstentionsChoice, Abstentions, {
    Counted => "counted",
    QuorumOnly => "quorum_only",
    Ignored => "ignored",
});
choice!(ExhaustedChoice, Exhausted, {
    Counted => "counted",
    Dropped => "dropped",
});
choice!(LastTwoChoice, LastTwo, {
    Runoff => "runoff",
    Final => "final",
});
choice!(BlockWeightChoice, BlockWeight, {
    Voters => "voters",
    Roster => "roster",
});

/// `/voteconfig`. The options that change the rules only exist when `rule_changes` is on
pub fn command(rule_changes: bool) -> poise::Command<Data, SerenityError> {
    if rule_changes { edit() } else { show() }
}

/// Show how votes are counted
#[poise::command(slash_command, rename = "voteconfig")]
async fn show(ctx: SerenityContext<'_>) -> Result<(), SerenityError> {
    let read = ChangeRules {
        requested_by: format!("discord /voteconfig @{}", ctx.author().name),
        ..ChangeRules::default()
    };
    answer(ctx, read).await
}

/// Show how votes are counted, or change it until the bot restarts
#[allow(clippy::too_many_arguments)]
#[poise::command(
    slash_command,
    guild_only,
    rename = "voteconfig",
    default_member_permissions = "MANAGE_GUILD"
)]
async fn edit(
    ctx: SerenityContext<'_>,
    #[description = "Is the weight of an abstention part of what an option needs more than half of"]
    abstentions: Option<AbstentionsChoice>,
    #[description = "Two options left, none has more than half: eliminate the weaker one or stop"]
    last_two_options: Option<LastTwoChoice>,
    #[description = "Does a ballot with no option left stay in what an option needs more than half of"]
    exhausted_ballots: Option<ExhaustedChoice>,
    #[description = "Size of the unaffiliated block"] unaffiliated_weight: Option<
        BlockWeightChoice,
    >,
    #[description = "Share of an organisation's roster that has to vote for it to count, 0 to 1"]
    #[min = 0.0]
    #[max = 1.0]
    internal_quorum: Option<f64>,
    #[description = "Share of the organisations that have to count for the vote to hold, 0 to 1"]
    #[min = 0.0]
    #[max = 1.0]
    coalition_quorum: Option<f64>,
    #[description = "How long a vote lasts when its first mail doesn't say"]
    #[min = 1]
    window_hours: Option<u32>,
    #[description = "Go back to the config file, the other options come on top"] reset: Option<
        bool,
    >,
) -> Result<(), SerenityError> {
    let change = ChangeRules {
        reset: reset.unwrap_or(false),
        internal_quorum,
        coalition_quorum,
        window_hours,
        unaffiliated_weight: unaffiliated_weight.map(Into::into),
        abstentions: abstentions.map(Into::into),
        exhausted_ballots: exhausted_ballots.map(Into::into),
        last_two_options: last_two_options.map(Into::into),
        requested_by: format!("discord /voteconfig @{}", ctx.author().name),
    };
    answer(ctx, change).await
}

/// Change or read the rules, then reply with them
async fn answer(ctx: SerenityContext<'_>, change: ChangeRules) -> Result<(), SerenityError> {
    let changes = change.changes();
    match ctx.data().voting.ask(change).await {
        Ok(rules) => ctx.say(text(&rules, changes)).await?,
        Err(SendError::HandlerError(error)) => {
            ctx.say(format!("Can't change the voting rules: {error}"))
                .await?
        }
        Err(error) => {
            ctx.say(format!("Can't reach the voting rules right now: {error}"))
                .await?
        }
    };
    Ok(())
}

/// The rules in config file syntax, with the ones that differ from the file marked
fn text(rules: &Rules, changed: bool) -> String {
    let rows = |config: &VotingConfig| -> [(&'static str, String); 7] {
        let quoted = |name: &str| format!("\"{name}\"");
        [
            (
                "abstentions",
                quoted(AbstentionsChoice::from(config.abstentions).name()),
            ),
            (
                "last_two_options",
                quoted(LastTwoChoice::from(config.last_two_options).name()),
            ),
            (
                "exhausted_ballots",
                quoted(ExhaustedChoice::from(config.exhausted_ballots).name()),
            ),
            (
                "unaffiliated_weight",
                quoted(BlockWeightChoice::from(config.unaffiliated_weight).name()),
            ),
            ("internal_quorum", config.internal_quorum.to_string()),
            ("coalition_quorum", config.coalition_quorum.to_string()),
            ("window_hours", config.window_hours.to_string()),
        ]
    };

    let mut swapped = false;
    let lines: Vec<String> = rows(&rules.current)
        .into_iter()
        .zip(rows(&rules.file))
        .map(|((name, current), (_, file))| {
            if current == file {
                format!("{name} = {current}")
            } else {
                swapped = true;
                format!("{name} = {current}  # config file: {file}")
            }
        })
        .collect();

    let title = if changed {
        "**Voting rules changed**, the next counts use them"
    } else {
        "**Voting rules**"
    };
    let note = if swapped {
        "Until the bot restarts, `/voteconfig reset:True` goes back to the config file"
    } else {
        "As in the config file"
    };
    format!("{title}\n```toml\n{}\n```{note}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_read_as_the_config_file() {
        let file = VotingConfig {
            internal_quorum: 0.5,
            coalition_quorum: 0.5,
            window_hours: 24,
            ..VotingConfig::default()
        };
        let rules = Rules {
            current: file.clone(),
            file: file.clone(),
        };
        assert_eq!(
            text(&rules, false),
            "**Voting rules**\n```toml\n\
             abstentions = \"counted\"\n\
             last_two_options = \"runoff\"\n\
             exhausted_ballots = \"counted\"\n\
             unaffiliated_weight = \"voters\"\n\
             internal_quorum = 0.5\n\
             coalition_quorum = 0.5\n\
             window_hours = 24\n\
             ```As in the config file"
        );

        let rules = Rules {
            current: VotingConfig {
                abstentions: Abstentions::QuorumOnly,
                last_two_options: LastTwo::Final,
                coalition_quorum: 0.75,
                ..file.clone()
            },
            file,
        };
        let text = text(&rules, true);
        assert!(text.starts_with("**Voting rules changed**"));
        assert!(text.contains("abstentions = \"quorum_only\"  # config file: \"counted\"\n"));
        assert!(text.contains("last_two_options = \"final\"  # config file: \"runoff\"\n"));
        assert!(text.contains("exhausted_ballots = \"counted\"\n"));
        assert!(text.contains("coalition_quorum = 0.75  # config file: 0.5\n"));
        assert!(text.ends_with("goes back to the config file"));
    }

    #[test]
    fn every_rule_has_its_choice() {
        for rule in [
            Abstentions::Counted,
            Abstentions::QuorumOnly,
            Abstentions::Ignored,
        ] {
            assert_eq!(Abstentions::from(AbstentionsChoice::from(rule)), rule);
        }
        for rule in [LastTwo::Runoff, LastTwo::Final] {
            assert_eq!(LastTwo::from(LastTwoChoice::from(rule)), rule);
        }
        for rule in [Exhausted::Counted, Exhausted::Dropped] {
            assert_eq!(Exhausted::from(ExhaustedChoice::from(rule)), rule);
        }
        for rule in [BlockWeight::Voters, BlockWeight::Roster] {
            assert_eq!(BlockWeight::from(BlockWeightChoice::from(rule)), rule);
        }
    }
}
