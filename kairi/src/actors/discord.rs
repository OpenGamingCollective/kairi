use std::sync::Arc;

use kameo::Actor;
use tokio::task::JoinHandle;

use crate::platforms::DiscordBot;

#[derive(Debug, Clone)]
pub struct DiscordArgs {
    pub token: String,
    // TODO: put other actors ref here
}
pub struct DiscordActor {
    bot: Arc<DiscordBot>,
    handle: JoinHandle<()>,
}

impl Actor for DiscordActor {
    type Args = DiscordArgs;
    type Error = anyhow::Error;

    async fn on_start(
        args: Self::Args,
        _actor_ref: kameo::prelude::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let bot = Arc::new(DiscordBot::build(&args.token).await?);

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
