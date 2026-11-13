//! Read a mirrored list from its index, and mail bodies from its local git

use std::path::PathBuf;

use mail_parser::Message;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    epoch::Epoch,
    errors::{MailingListError, Result},
    index,
    message::{self, Mail},
    thread::{Thread, ThreadSummary},
};

/// A public-inbox list, read from `dir`, never written to
pub struct Reader {
    dir: PathBuf,
}

/// The list and a summary of each thread, sorted from the oldest thread on
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct List {
    /// From the list's manifest
    pub description: String,
    pub threads: Vec<ThreadSummary>,
}

impl Reader {
    /// Read the list the mirror keeps in `dir`
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The list, empty until the mirror indexed it
    #[instrument(level = "debug", skip_all, fields(dir = %self.dir.display()))]
    pub fn fetch_list(&self) -> Result<List> {
        Ok(index::read_list(&self.dir)?.unwrap_or_default())
    }

    /// A thread of the list, by the sha-derived uid of its root
    #[instrument(level = "debug", skip_all, fields(dir = %self.dir.display(), uid = uid))]
    pub fn fetch_thread(&self, uid: &str) -> Result<Option<Thread>> {
        index::read_thread(&self.dir, uid)
    }

    /// Read a whole mail from the mirror
    #[instrument(
        level = "debug",
        skip_all,
        fields(dir = %self.dir.display(), epoch = mail.epoch, blob = %mail.blob),
        err(level = "warn")
    )]
    pub fn message(&self, mail: &Mail) -> Result<Message<'static>> {
        let epoch = Epoch::open_readonly(&self.dir, mail.epoch)?
            .ok_or(MailingListError::EpochNotFound(mail.epoch))?;
        message::parse(epoch.blob(mail.blob)?)
    }
}
