//! `/voteconfig` shows the voting rules

use kairi_config::VotingConfig;

use super::{SerenityContext, SerenityError};

/// Show how votes are counted
#[poise::command(slash_command, rename = "voteconfig")]
pub async fn voteconfig(ctx: SerenityContext<'_>) -> Result<(), SerenityError> {
    ctx.say(text(&ctx.data().rules)).await?;
    Ok(())
}

fn text(config: &VotingConfig) -> String {
    format!(
        "**Voting rules**\n```toml\ninternal_quorum = {}\ncoalition_quorum = {}\nwindow_hours = {}\n```",
        config.internal_quorum, config.coalition_quorum, config.window_hours
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rules_read_as_the_config_file() {
        assert_eq!(
            text(&VotingConfig::default()),
            "**Voting rules**\n```toml\n\
             internal_quorum = 0.5\n\
             coalition_quorum = 0.5\n\
             window_hours = 24\n\
             ```"
        );
    }
}
