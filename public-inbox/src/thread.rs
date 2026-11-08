//! Handle scraping of a mailing list's thread
use crate::errors::{MailingListError, Result};
use crate::message::{author_of, fetch_messages, mail_dt_to_chrono, mbox_url};
use chrono::{DateTime, Utc};
use mail_parser::Message;
use reqwest::{Client, Url};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Representation of a mailing list thread
#[derive(Debug, Clone)]
pub struct Thread {
    uid: String,
    name: String,
    author: String,
    date: DateTime<Utc>,
    url: Url,
    root: Message<'static>,
}

impl Thread {
    pub fn uid(&self) -> &str {
        &self.uid
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn author(&self) -> &str {
        &self.author
    }

    pub fn date(&self) -> &DateTime<Utc> {
        &self.date
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    /// Initiating email of the thread
    pub fn root(&self) -> &Message<'static> {
        &self.root
    }

    pub fn mbox_url(&self) -> Result<Url> {
        mbox_url(&self.url)
    }

    pub async fn get_messages(&self, client: &Client) -> Result<Vec<Message<'static>>> {
        fetch_messages(client, &self.mbox_url()?).await
    }

    pub fn from_url(url: &str, messages: &[Message<'_>]) -> Result<Self> {
        let mut url = Url::parse(url)?;
        url.set_fragment(None);
        url.set_query(None);

        let root = root_of(messages).ok_or(MailingListError::NoFirstMessageFound)?;
        let date = mail_dt_to_chrono(*root.date().ok_or(MailingListError::NoFirstMessageDate)?)
            .ok_or(MailingListError::DtToChronoError)?;

        Ok(Self {
            uid: Self::uid_of(root),
            name: root.subject().unwrap_or_default().to_string(),
            author: author_of(root),
            date,
            url,
            root: root.clone().into_owned(),
        })
    }
    fn uid_of(message: &Message<'_>) -> String {
        let subject = message.subject().unwrap_or_default();
        let timestamp = message.date().map(|date| date.to_timestamp()).unwrap_or(0);

        let digest = Sha256::digest(format!("{subject}|{timestamp}").as_bytes());
        let hex = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        hex[..12].to_string()
    }
}

fn root_of<'a>(messages: &'a [Message<'a>]) -> Option<&'a Message<'a>> {
    let ids: HashSet<&str> = messages.iter().filter_map(|m| m.message_id()).collect();

    messages
        .iter()
        .find(|m| {
            let in_reply_to = m.in_reply_to().as_text().unwrap_or_default();
            let references = m.references().as_text().unwrap_or_default();
            !in_reply_to
                .split_whitespace()
                .chain(references.split_whitespace())
                .any(|id| ids.contains(id))
        })
        .or_else(|| messages.first())
}
