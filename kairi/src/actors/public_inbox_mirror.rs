//! Keeps the local mirror of each mailing list in sync

use std::{
    sync::{Arc, PoisonError},
    time::Duration,
};

use kameo::{Actor, prelude::ActorRef};
use public_inbox::MailingList;
use tokio::{task::JoinSet, time::MissedTickBehavior};
use tracing::warn;

use crate::types::{MailingListEnum, MailingLists};

const LISTS_URL: &str = "https://lists.opengamingcollective.org";
const INTERVAL: Duration = Duration::from_secs(60);

pub struct PublicInboxMirror {
    /// Aborted when the actor is dropped
    _tasks: JoinSet<()>,
}

#[derive(Debug, Clone)]
pub struct PublicInboxMirrorArgs {
    pub repositories: Vec<MailingListEnum>,
    pub lists: MailingLists,
}

impl Actor for PublicInboxMirror {
    type Args = PublicInboxMirrorArgs;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "public-inbox-mirror"
    }

    async fn on_start(args: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        // Spawn one task for each list to mirror
        let mut tasks = JoinSet::new();
        for repository in args.repositories {
            tasks.spawn(mirror(repository, args.lists.clone()));
        }
        Ok(Self { _tasks: tasks })
    }
}

/// Open the list's mirror once, then fetch it every minute
async fn mirror(list: MailingListEnum, lists: MailingLists) {
    let mut ticks = tokio::time::interval(INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        ticks.tick().await;
        let opened = lists
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&list)
            .cloned();

        let result = match opened {
            Some(mirror) => mirror.fetch().await,
            None => {
                // Relative to the working directory, the service's data dir
                let dir = format!("public-inbox/{list}");
                MailingList::build(&format!("{LISTS_URL}/{list}"), dir, "kairi")
                    .await
                    .map(|mirror| {
                        lists
                            .write()
                            .unwrap_or_else(PoisonError::into_inner)
                            .insert(list, Arc::new(mirror));
                    })
            }
        };

        if let Err(error) = result {
            warn!(%list, %error, "mirroring failed, retrying next minute");
        }
    }
}
