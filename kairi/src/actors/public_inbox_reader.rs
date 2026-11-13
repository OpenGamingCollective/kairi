//! Reads the mirrored lists from their index on the disk

use std::{path::PathBuf, time::Instant};

use kameo::prelude::*;
use public_inbox::{List, MailingListError, Reader, Thread};
use tracing::{Span, info, instrument};

use crate::types::MailingListEnum;

pub struct PublicInboxReader {
    /// Each list is mirrored in `<dir>/<list>`
    dir: PathBuf,
}

/// A list
#[derive(Debug, Clone)]
pub struct FetchList {
    pub list: MailingListEnum,
    /// Who asked, for the logs, e.g. `discord /threads @user`
    pub requested_by: String,
}

/// A thread by its uid
#[derive(Debug, Clone)]
pub struct FetchThread {
    pub list: MailingListEnum,
    pub uid: String,
    /// Who asked, for the logs, e.g. `discord /thread @user`
    pub requested_by: String,
}

impl Actor for PublicInboxReader {
    type Args = PathBuf;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "public-inbox-reader"
    }

    async fn on_start(dir: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        Ok(Self { dir })
    }
}

impl Message<FetchList> for PublicInboxReader {
    type Reply = public_inbox::Result<List>;

    #[instrument(
        name = "FetchList",
        skip_all,
        fields(%list, requested_by = %requested_by),
        err(level = "warn")
    )]
    async fn handle(
        &mut self,
        FetchList { list, requested_by }: FetchList,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let started = Instant::now();
        // File reads are blocking
        let reader = self.reader(list);
        let span = Span::current();
        let read = tokio::task::spawn_blocking(move || span.in_scope(|| reader.fetch_list()))
            .await
            .map_err(MailingListError::from)??;

        let elapsed_ms = started.elapsed().as_millis();
        info!(threads = read.threads.len(), elapsed_ms, "served");
        Ok(read)
    }
}

impl Message<FetchThread> for PublicInboxReader {
    type Reply = public_inbox::Result<Option<Thread>>;

    #[instrument(
        name = "FetchThread",
        skip_all,
        fields(%list, uid = %uid, requested_by = %requested_by),
        err(level = "warn")
    )]
    async fn handle(
        &mut self,
        FetchThread {
            list,
            uid,
            requested_by,
        }: FetchThread,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let started = Instant::now();
        let reader = self.reader(list);
        let span = Span::current();
        let thread =
            tokio::task::spawn_blocking(move || span.in_scope(|| reader.fetch_thread(&uid)))
                .await
                .map_err(MailingListError::from)??;

        let elapsed_ms = started.elapsed().as_millis();
        match &thread {
            Some(thread) => info!(mails = thread.mails().len(), elapsed_ms, "served"),
            None => info!(elapsed_ms, "no such thread"),
        }
        Ok(thread)
    }
}

impl PublicInboxReader {
    fn reader(&self, list: MailingListEnum) -> Reader {
        Reader::open(self.dir.join(list.to_string()))
    }
}
