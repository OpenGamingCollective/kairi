//! Keeps the local mirror of each mailing list in sync

use std::{path::PathBuf, time::Duration};

use kameo::prelude::*;
use public_inbox::Mirror;
use tokio::{task::AbortHandle, time::MissedTickBehavior};
use tracing::{Instrument, error};

use crate::types::{LISTS_URL, MailingListEnum};

pub struct PublicInboxMirror;

#[derive(Debug, Clone)]
pub struct PublicInboxMirrorArgs {
    pub repositories: Vec<MailingListEnum>,
    /// Each list is mirrored in `<dir>/<list>`
    pub dir: PathBuf,
}

impl Actor for PublicInboxMirror {
    type Args = PublicInboxMirrorArgs;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "public-inbox-mirror-pool"
    }

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        for repository in args.repositories {
            let args = PublicInboxMirrorWorkerArgs {
                list: repository,
                dir: args.dir.join(repository.to_string()),
            };
            PublicInboxMirrorWorker::supervise(&actor_ref, args)
                .spawn()
                .await;
        }
        Ok(Self)
    }
}

#[derive(Debug, Clone)]
struct PublicInboxMirrorWorkerArgs {
    list: MailingListEnum,
    dir: PathBuf,
}

struct PublicInboxMirrorWorker {
    /// Aborted with the actor, restarted workers never leave their sync behind
    sync: AbortHandle,
}

impl Drop for PublicInboxMirrorWorker {
    fn drop(&mut self) {
        self.sync.abort();
    }
}

impl Actor for PublicInboxMirrorWorker {
    type Args = PublicInboxMirrorWorkerArgs;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "public-inbox-mirror-worker"
    }

    async fn on_start(args: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        let PublicInboxMirrorWorkerArgs { list, dir } = args;
        let url = format!("{LISTS_URL}/{list}");

        let inbox = Mirror::open(&url, dir, crate::USER_AGENT)?;

        let sync = tokio::spawn(mirror(list, inbox));
        let abort = sync.abort_handle();

        let worker = actor_ref.downgrade();
        tokio::spawn(async move {
            if let Err(error) = sync.await
                && error.is_panic()
            {
                error!(%list, "mirroring panicked, restarting the worker");
                worker.kill();
            }
        });
        Ok(Self { sync: abort })
    }
}

/// Fetch the list's mirror every minute, the first fetch clones it if it's absent
async fn mirror(list: MailingListEnum, mut inbox: Mirror) {
    let span = tracing::info_span!("public-inbox-mirror-worker", %list);
    async move {
        let mut ticks = tokio::time::interval(Duration::from_secs(60));
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            ticks.tick().await;
            if let Err(error) = inbox.fetch().await {
                error!(%error, "mirroring failed, retrying next minute");
            }
        }
    }
    .instrument(span)
    .await;
}
