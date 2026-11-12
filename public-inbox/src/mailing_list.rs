//! Handle mirroring a mailing list

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use mail_parser::Message;
use reqwest::Client;

use crate::{errors::Result, manifest, message::Mail, mirror::Mirror, thread::Thread};

/// Represent a mailing list
#[derive(Debug)]
pub struct MailingList {
    url: String,
    client: Client,
    name: String,
    description: String,
    mirror: Arc<Mutex<Mirror>>,
    /// Only one fetch at a time touches the repositories
    syncing: Arc<Mutex<()>>,
}

impl MailingList {
    /// Mirror the list at `url` into `dir`, a new mirror starts with everything seen
    pub async fn build(url: &str, dir: impl Into<PathBuf>, client_name: &str) -> Result<Self> {
        let url = url.trim_end_matches('/');
        let name = url.rsplit('/').next().unwrap_or_default().to_string();
        let url = format!("{url}/");

        let client = Client::builder().user_agent(client_name).build()?;
        let (description, epochs) = manifest::fetch(&client, &url).await?;

        let list = Self {
            mirror: Arc::new(Mutex::new(Mirror::open(url.clone(), dir.into())?)),
            url,
            client,
            name,
            description,
            syncing: Arc::default(),
        };
        list.update(epochs).await?;
        list.lock().finish_first_start()?;

        Ok(list)
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    /// Name of the mailing list, taken from the last URL path segment
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Description of the mailing list, taken from its manifest
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Get the threads, sorted by the date of their root
    pub fn threads(&self) -> Vec<Thread> {
        self.lock().threads.all()
    }

    /// Get a thread using its uid
    pub fn get_thread(&self, uid: &str) -> Option<Thread> {
        self.lock().threads.get(uid).cloned()
    }

    /// Bring the mirror up to date with the list
    pub async fn fetch(&self) -> Result<()> {
        let (_, epochs) = manifest::fetch(&self.client, &self.url).await?;
        self.update(epochs).await
    }

    /// Mails not marked as seen yet, in archive order
    pub fn pending(&self) -> Vec<Mail> {
        self.lock().unseen.clone()
    }

    /// Persist that `mail` and the ones before it were handled, a no-op if they already were
    pub fn mark_seen(&self, mail: &Mail) -> Result<()> {
        self.lock().mark_seen(mail)
    }

    /// Read a whole mail from the mirror
    pub fn message(&self, mail: &Mail) -> Result<Message<'static>> {
        self.lock().message(mail)
    }

    fn lock(&self) -> MutexGuard<'_, Mirror> {
        lock(&self.mirror)
    }

    /// Fetch without holding the lock, then index
    async fn update(&self, epochs: Vec<u32>) -> Result<()> {
        let syncing = self.syncing.clone();
        let mirror = self.mirror.clone();
        let url = self.url.clone();

        // Locked in the task, so a cancelled sync keeps it until the fetch ends
        tokio::task::spawn_blocking(move || {
            let _syncing = lock(&syncing);
            let epochs = lock(&mirror).to_fetch(&epochs)?;
            for epoch in &epochs {
                epoch.fetch(&url)?;
            }

            let mut mirror = lock(&mirror);
            mirror.add(epochs);
            mirror.index()
        })
        .await?
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
