use kairi_config::Config;
use kameo::{
    Actor,
    actor::{ActorRef, Spawn},
    supervision::{RestartPolicy, SupervisionStrategy},
};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use strum::VariantArray;

use crate::{
    actors::{
        discord::{DiscordActor, DiscordArgs},
        public_inbox_mirror::{PublicInboxMirror, PublicInboxMirrorArgs},
        public_inbox_reader::PublicInboxReader,
        voting::{Voting, VotingArgs},
    },
    types::MailingListEnum,
};

#[derive(Default)]
pub struct Kairi {
    pub config: Arc<OnceLock<Config>>,
    pub discord_ref: Arc<OnceLock<ActorRef<DiscordActor>>>,
    pub reader_ref: Arc<OnceLock<ActorRef<PublicInboxReader>>>,
    pub voting_ref: Arc<OnceLock<ActorRef<Voting>>>,
}

#[derive(Debug, Clone, Default)]
pub struct KairiArg {
    pub config: Config,
    /// State directory, from `--data-dir`
    pub data_dir: PathBuf,
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
        let public_inbox_dir = args.data_dir.join("public-inbox");

        let _ = kairi.config.set(args.config.clone());

        let _public_inbox_mirror = PublicInboxMirror::supervise(
            &actor_ref,
            PublicInboxMirrorArgs {
                repositories: MailingListEnum::VARIANTS.to_vec(),
                dir: public_inbox_dir.clone(),
            },
        )
        .restart_policy(RestartPolicy::Permanent)
        .restart_limit(5, Duration::from_secs(10))
        .spawn()
        .await;

        let public_inbox_reader = PublicInboxReader::supervise(&actor_ref, public_inbox_dir)
            .restart_policy(RestartPolicy::Permanent)
            .restart_limit(5, Duration::from_secs(10))
            .spawn()
            .await;

        let voting = Voting::supervise(
            &actor_ref,
            VotingArgs {
                reader: public_inbox_reader.clone(),
                config: args.config.voting.clone(),
            },
        )
        .restart_policy(RestartPolicy::Permanent)
        .restart_limit(5, Duration::from_secs(10))
        .spawn()
        .await;

        // Started last, since its commands talk to the other actors
        let discord = DiscordActor::supervise(
            &actor_ref,
            DiscordArgs {
                rule_changes: args.config.voting.rule_changes,
                token: args.config.discord.token,
                voting: voting.clone(),
                reader: public_inbox_reader.clone(),
            },
        )
        .restart_policy(RestartPolicy::Permanent)
        .restart_limit(5, Duration::from_secs(10))
        .spawn()
        .await;

        let _ = kairi.reader_ref.set(public_inbox_reader);

        let _ = kairi.voting_ref.set(voting);

        let _ = kairi.discord_ref.set(discord);

        Ok(kairi)
    }
}
