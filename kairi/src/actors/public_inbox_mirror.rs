use std::time::Duration;

use kameo::prelude::*;
use tokio::{
    task::{self, JoinHandle},
    time::sleep,
};
use tracing::{Instrument, info};

use crate::types::MailingListEnum;

#[derive(Default)]
pub struct PublicInboxMirror {
    pub handles: Vec<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
pub struct PublicInboxMirrorArgs {
    pub repositories: Vec<MailingListEnum>,
}

impl Actor for PublicInboxMirror {
    type Args = PublicInboxMirrorArgs;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "public-inbox-mirror-pool"
    }

    async fn on_start(
        args: Self::Args,
        actor_ref: kameo::prelude::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let actor = Self::default();
        for repository in args.repositories {
            PublicInboxMirrorWorker::supervise(&actor_ref, repository)
                .spawn()
                .await;
        }
        Ok(actor)
    }
}

struct PublicInboxMirrorWorker {
    list: MailingListEnum,
    handle: JoinHandle<()>,
}

impl Actor for PublicInboxMirrorWorker {
    type Args = MailingListEnum;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "public-inbox-mirror-worker"
    }
    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let span = tracing::info_span!("public-inbox-mirror-worker", list = %args);

        let sync_task = task::spawn(
            async move {
                loop {
                    info!("do sync");
                    sleep(Duration::from_secs(2)).await
                }
            }
            .instrument(span),
        );

        Ok(Self {
            list: args,
            handle: sync_task,
        })
    }
}
