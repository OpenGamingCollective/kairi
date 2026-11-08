//! Fetching and parsing of raw mailing list messages

use chrono::{DateTime, Utc};
use flate2::read::MultiGzDecoder;
use mail_parser::{
    DateTime as MailDateTime, Message, MessageParser, mailbox::mbox::MessageIterator,
};
use reqwest::{Client, Url};
use std::io::BufReader;

use crate::errors::{MailingListError, Result};

pub fn mbox_url(url: &Url) -> Result<Url> {
    Ok(url.join("../t.mbox.gz")?)
}

pub async fn fetch_messages(client: &Client, mbox_url: &Url) -> Result<Vec<Message<'static>>> {
    let bytes = client
        .get(mbox_url.clone())
        .header("Cache-Control", "no-cache")
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let decoder = MultiGzDecoder::new(bytes.as_ref());

    let reader = BufReader::new(decoder);

    let parser = MessageParser::default();

    let mut messages: Vec<Message> = Vec::new();

    for entry in MessageIterator::new(reader) {
        let entry = entry?;
        let message = parser
            .parse(entry.contents())
            .ok_or(MailingListError::MailParseFailed)?;

        messages.push(message.into_owned());
    }

    if messages.is_empty() {
        return Err(MailingListError::ThreadEmpty);
    }

    Ok(messages)
}

/// Thread author, falls back to the email
pub fn author_of(message: &Message<'_>) -> String {
    if let Some(addr) = message.from().and_then(|address| address.first()) {
        if let Some(name) = addr.name.as_deref().filter(|name| !name.is_empty()) {
            return name.to_string();
        }
        if let Some(address) = addr.address.as_deref() {
            return address.to_string();
        }
    }
    sender_of(message).unwrap_or_default()
}

/// Original sender
pub fn sender_of(message: &Message<'_>) -> Option<String> {
    if let Some(sender) = message
        .header("X-MailFrom")
        .and_then(|value| value.as_text())
    {
        return Some(sender.to_string());
    }
    message
        .from()
        .and_then(|address| address.first())
        .and_then(|addr| addr.address.as_deref())
        .map(str::to_string)
}

/// Convert a mail_parser DateTime into a chrono DateTime
pub fn mail_dt_to_chrono(dt: MailDateTime) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(dt.to_timestamp(), 0)
}
