use kairi_config::Config;
use kameo::{
    Actor,
    actor::{ActorRef, Spawn},
    supervision::{RestartPolicy, SupervisionStrategy},
};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use crate::{
    actors::{
        discord::{DiscordActor, DiscordArgs},
        public_inbox_mirror::{PublicInboxMirror, PublicInboxMirrorArgs},
    },
    types::{MailingListEnum, MailingLists},
};

#[derive(Default)]
pub struct Kairi {
    pub config: Arc<OnceLock<Config>>,
    pub discord_ref: Arc<OnceLock<ActorRef<DiscordActor>>>,
}

#[derive(Debug, Clone, Default)]
pub struct KairiArg {
    pub config: Config,
}

impl Actor for Kairi {
    type Args = KairiArg;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn name() -> &'static str {
        "kairi"
    }

    fn supervision_strategy() -> SupervisionStrategy {
        // No need to restart every actors if one fails
        SupervisionStrategy::OneForOne
    }

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let kairi = Self::default();

        let _ = kairi.config.set(args.config.clone());

        let discord = DiscordActor::supervise(
            &actor_ref,
            DiscordArgs {
                token: args.config.discord.token,
            },
        )
        .restart_policy(RestartPolicy::Permanent)
        .restart_limit(5, Duration::from_secs(10))
        .spawn()
        .await;

        let _public_inbox_mirror = PublicInboxMirror::supervise(
            &actor_ref,
            PublicInboxMirrorArgs {
                repositories: [MailingListEnum::Governance].to_vec(),
                // Shared with the actors that read the lists
                lists: MailingLists::default(),
            },
        )
        .restart_policy(RestartPolicy::Permanent)
        .restart_limit(5, Duration::from_secs(10))
        .spawn()
        .await;

        let _ = kairi.discord_ref.set(discord);

        Ok(kairi)
    }
}
