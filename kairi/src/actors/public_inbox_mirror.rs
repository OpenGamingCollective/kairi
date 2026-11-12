use kameo::Actor;
use tokio::task::JoinHandle;

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
        "public-inbox-mirror"
    }

    async fn on_start(
        args: Self::Args,
        actor_ref: kameo::prelude::ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let mut actor = Self::default();
        // Spawn one task for each list to mirror
        let mut handles: Vec<JoinHandle<()>> = Vec::new();

        for repository in args.repositories {
            handles.push(tokio::spawn(async move {
                // Mirror task for said repo
            }));
        }
        actor.handles = handles;
        Ok(actor)
    }
}
