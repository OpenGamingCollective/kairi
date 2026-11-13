//! Local copy of a mailing list, kept in sync with upstream and indexed

use std::path::PathBuf;

use reqwest::Client;
use tracing::{Span, debug, info, instrument};

use crate::{epoch::Epoch, errors::Result, index, manifest};

/// A public-inbox list, mirrored to `dir`
pub struct Mirror {
    url: String,
    dir: PathBuf,
    client: Client,
    /// Epochs of the last fetch, owned handles
    epochs: Vec<Epoch>,
    /// The index was checked file by file since the process started
    verified: bool,
}

impl Mirror {
    /// Mirror the list at `url` into `dir`, nothing is fetched until `fetch`
    pub fn open(url: &str, dir: impl Into<PathBuf>, client_name: &str) -> Result<Self> {
        Ok(Self {
            url: format!("{}/", url.trim_end_matches('/')),
            dir: dir.into(),
            client: Client::builder().user_agent(client_name).build()?,
            epochs: Vec::new(),
            verified: false,
        })
    }

    /// Fetch the manifest, git-fetch the epochs it lists from the newest one on, then reindex
    #[instrument(level = "debug", skip_all)]
    pub async fn fetch(&mut self) -> Result<()> {
        let (description, numbers) = manifest::fetch(&self.client, &self.url).await?;
        debug!(epochs = ?numbers, "manifest fetched");

        // Cloned handles, the fetch runs off the async runtime
        let url = self.url.clone();
        let dir = self.dir.clone();
        let epochs = self.epochs.clone();
        // Only a power loss breaks the index, and it always restarts the process
        let verify = !self.verified;
        // Blocking tasks don't inherit the caller's span, the list it logs
        let span = Span::current();

        let fetched = tokio::task::spawn_blocking(move || -> Result<Vec<Epoch>> {
            let _span = span.enter();
            // Older epochs are only fetched on the first sync of the process, purging rewrites them
            let newest = epochs.last().map_or(0, |epoch| epoch.number);
            let mut epochs = epochs;
            for &number in &numbers {
                if number >= newest && !epochs.iter().any(|epoch| epoch.number == number) {
                    if !epochs.is_empty() {
                        info!(epoch = number, "new epoch");
                    }
                    epochs.push(Epoch::open(&dir, number)?);
                }
            }
            for epoch in &epochs {
                if epoch.number >= newest {
                    epoch.fetch(&url)?;
                }
            }
            index::reindex(&dir, &url, &description, verify)?;
            Ok(epochs)
        })
        .await??;

        self.epochs = fetched;
        self.verified = true;
        Ok(())
    }

    /// Rebuild the index from the git already on disk if it changed or is broken, `true` if it did
    ///
    /// Blocking and reads the whole index, `fetch` already does it after each fetch
    pub fn reindex(&self, description: &str) -> Result<bool> {
        index::reindex(&self.dir, &self.url, description, true)
    }
}
