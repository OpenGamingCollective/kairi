//! Parsing of raw mailing list messages

use std::{fmt::Write, sync::LazyLock};

use chrono::{DateTime, Utc};
use gix::ObjectId;
use mail_parser::{HeaderForm, Message, MessageParser};
use serde::{Deserialize, Serialize};

use crate::errors::{MailingListError, Result};

/// Mailman header carrying the envelope sender
const MAIL_FROM: &str = "X-MailFrom";

/// Headers only, used to index mails
static HEADERS: LazyLock<MessageParser> = LazyLock::new(|| {
    MessageParser::new()
        .unknown_headers(HeaderForm::Ignore)
        .header(MAIL_FROM, HeaderForm::Text)
});

static FULL: LazyLock<MessageParser> =
    LazyLock::new(|| MessageParser::new().header(MAIL_FROM, HeaderForm::Text));

/// Headers of an archived mail, the body stays in the mirror
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mail {
    pub message_id: String,
    pub subject: String,
    pub author: String,
    pub sender: Option<String>,
    pub date: DateTime<Utc>,
    pub url: String,
    pub(crate) epoch: u32,
    pub(crate) blob: ObjectId,
    /// Ids this mail replies to, closest first
    pub(crate) parents: Vec<String>,
}

/// `None` when the mail has no Message-ID
pub(crate) fn index(
    raw: &[u8],
    list_url: &str,
    epoch: u32,
    blob: ObjectId,
    commit_time: i64,
) -> Option<Mail> {
    let message = HEADERS.parse_headers(raw)?;
    let message_id = message.message_id()?;

    let parents = message
        .in_reply_to()
        .into_iter()
        .chain(message.references())
        .flat_map(|ids| ids.iter().rev())
        .map(str::to_string)
        .collect();

    let date = message
        .date()
        .and_then(|date| DateTime::from_timestamp(date.to_timestamp(), 0))
        .or_else(|| DateTime::from_timestamp(commit_time, 0))
        .unwrap_or_default();

    Some(Mail {
        message_id: message_id.to_string(),
        subject: message.subject().unwrap_or_default().to_string(),
        author: author_of(&message),
        sender: sender_of(&message),
        date,
        url: format!("{list_url}{}/", escape_message_id(message_id)),
        epoch,
        blob,
        parents,
    })
}

pub(crate) fn parse(raw: Vec<u8>) -> Result<Message<'static>> {
    FULL.parse_owned(raw)
        .ok_or(MailingListError::MailParseFailed)
}

/// Name, falls back to the email
fn author_of(message: &Message<'_>) -> String {
    message
        .from()
        .and_then(|from| from.first())
        .and_then(|mailbox| {
            mailbox
                .name()
                .filter(|name| !name.is_empty())
                .or(mailbox.address())
        })
        .map(str::to_string)
        .or_else(|| sender_of(message))
        .unwrap_or_default()
}

/// Original sender
fn sender_of(message: &Message<'_>) -> Option<String> {
    message
        .headers()
        .value(MAIL_FROM)
        .and_then(|value| value.as_text())
        .or_else(|| message.from()?.first()?.address())
        .map(|sender| sender.trim().to_string())
}

/// Escape a Message-ID the way public-inbox does in its URLs
fn escape_message_id(message_id: &str) -> String {
    let mut escaped = String::with_capacity(message_id.len());
    for byte in message_id.bytes() {
        if byte.is_ascii_alphanumeric() || b"!$&'()*+,;=:@_.~-".contains(&byte) {
            escaped.push(byte as char);
        } else {
            let _ = write!(escaped, "%{byte:02X}");
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_raw(raw: &[u8], commit_time: i64) -> Option<Mail> {
        let null = ObjectId::null(gix::hash::Kind::Sha1);
        index(raw, "https://l/list/", 0, null, commit_time)
    }

    #[test]
    fn message_id_is_escaped_for_urls() {
        assert_eq!(escape_message_id("a-b.c@d_e"), "a-b.c@d_e");
        assert_eq!(escape_message_id("a/b c@d"), "a%2Fb%20c@d");
    }

    #[test]
    fn index_reads_the_headers() {
        let raw = b"From: Alice <alice@example.org>\r\n\
            X-MailFrom: alice@lists.example.org\r\n\
            Subject: hello\r\n\
            Date: Sat, 20 Nov 2021 14:22:01 -0800\r\n\
            Message-ID: <2@example.org>\r\n\
            In-Reply-To: <1@example.org>\r\n\
            References: <0@example.org> <1@example.org>\r\n\
            \r\n\
            body";

        let mail = index_raw(raw, 0).unwrap();
        assert_eq!(mail.message_id, "2@example.org");
        assert_eq!(mail.subject, "hello");
        assert_eq!(mail.author, "Alice");
        assert_eq!(mail.sender.as_deref(), Some("alice@lists.example.org"));
        assert_eq!(mail.date.timestamp(), 1_637_446_921);
        assert_eq!(mail.url, "https://l/list/2@example.org/");
        assert_eq!(
            mail.parents,
            ["1@example.org", "1@example.org", "0@example.org"]
        );
    }

    #[test]
    fn unparsable_date_falls_back_to_the_commit_time() {
        let mail = index_raw(b"Message-ID: <a@b>\r\nDate: garbage\r\n\r\nbody", 42);
        assert_eq!(mail.unwrap().date.timestamp(), 42);
    }

    #[test]
    fn mail_without_message_id_is_not_indexed() {
        assert!(index_raw(b"Subject: hello\r\n\r\nbody", 0).is_none());
    }
}
