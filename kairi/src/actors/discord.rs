use std::sync::Arc;

use kairi_config::VotingConfig;
use kameo::Actor;
use tokio::task::JoinHandle;

use crate::platforms::{Data, DiscordBot};

#[derive(Debug, Clone)]
pub struct DiscordArgs {
    pub token: String,
    pub rules: VotingConfig,
}
pub struct DiscordActor {
    bot: Arc<DiscordBot>,
    handle: JoinHandle<()>,
}

impl Actor for DiscordActor {
    type Args = DiscordArgs;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "discord-bot"
    }

    async fn on_start(
        args: Self::Args,
        actor_ref: kameo::prelude::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        actor_ref.register(<Self as Actor>::name())?;
        let bot = Arc::new(DiscordBot::build(&args.token, Data { rules: args.rules }).await?);

        let clone = bot.client.clone();

        let handle = tokio::spawn(async move {
            let mut bot = clone.lock().await;
            let _ = bot.start().await;
        });

        Ok(Self { bot, handle })
    }
    async fn on_stop(
        &mut self,
        _actor_ref: kameo::prelude::WeakActorRef<Self>,
        _reason: kameo::prelude::ActorStopReason,
    ) -> Result<(), Self::Error> {
        self.handle.abort();
        Ok(())
    }
}
