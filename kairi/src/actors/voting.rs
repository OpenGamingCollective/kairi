//! Actor that counts the votes on a mailing list thread

use chrono::{TimeDelta, Utc};
use kairi_config::VotingConfig;
use kameo::{error::SendError, prelude::*};
use public_inbox::{MailingListError, Thread};
use thiserror::Error;
use tracing::{info, instrument};
use voting::{MotionError, Policy, Reply, Roster, Tally};

use crate::{
    actors::public_inbox_reader::{FetchMessages, FetchThread, FindThread, PublicInboxReader},
    types::MailingListEnum,
};

pub struct Voting {
    reader: ActorRef<PublicInboxReader>,
    config: VotingConfig,
}

#[derive(Debug, Clone)]
pub struct VotingArgs {
    pub reader: ActorRef<PublicInboxReader>,
    pub config: VotingConfig,
}

/// Count the votes on a thread so far
#[derive(Debug, Clone)]
pub struct CountVotes {
    pub list: MailingListEnum,
    /// The thread's uid, a link to it, or the Message-ID of any mail in it
    pub thread: String,
    /// Who's asking, for the logs, e.g. `discord /votes @user`
    pub requested_by: String,
}

/// The count for a thread
#[derive(Debug, Clone)]
pub struct Count {
    pub list: MailingListEnum,
    /// The thread's uid
    pub uid: String,
    pub name: String,
    pub url: String,
    pub tally: Tally,
}

#[derive(Debug, Error)]
pub enum VotingError {
    #[error("no roster is configured, `voting.roster_path` is not set")]
    NoRoster,

    #[error("the roster can't be read: {0}")]
    Roster(String),

    #[error("no such thread")]
    NoSuchThread,

    #[error("this thread is not a vote, {0}")]
    NotAVote(#[from] MotionError),

    #[error("the mailing list can't be read: {0}")]
    Reader(String),
}

impl<M> From<SendError<M, MailingListError>> for VotingError {
    fn from(error: SendError<M, MailingListError>) -> Self {
        Self::Reader(error.to_string())
    }
}

impl Actor for Voting {
    type Args = VotingArgs;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "voting"
    }

    async fn on_start(args: Self::Args, _actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        Ok(Self {
            reader: args.reader,
            config: args.config,
        })
    }
}

impl Message<CountVotes> for Voting {
    type Reply = Result<Count, VotingError>;

    #[instrument(
        name = "CountVotes",
        skip_all,
        fields(%list, thread = %thread, requested_by = %requested_by),
        err(level = "warn")
    )]
    async fn handle(
        &mut self,
        CountVotes {
            list,
            thread,
            requested_by,
        }: CountVotes,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        // Re-read on every count so it can change without a restart
        let roster = self.roster().await?;

        let thread = self
            .thread(list, &thread, &requested_by)
            .await?
            .ok_or(VotingError::NoSuchThread)?;
        let messages = self
            .reader
            .ask(FetchMessages {
                list,
                mails: thread.mails().to_vec(),
                requested_by,
            })
            .await?;

        let mut mails = thread
            .mails()
            .iter()
            .zip(&messages)
            .map(|(mail, message)| Reply {
                author: mail.author.clone(),
                sender: mail.sender.clone(),
                date: mail.date,
                url: mail.url.clone(),
                body: message.body_text(0).unwrap_or_default().into_owned(),
            });
        let motion = mails.next().ok_or(VotingError::NoSuchThread)?;
        let replies: Vec<Reply> = mails.collect();

        let policy = Policy {
            internal_quorum: self.config.internal_quorum,
            coalition_quorum: self.config.coalition_quorum,
            window: TimeDelta::hours(self.config.window_hours.into()),
        };
        let tally = voting::tally(
            &motion.body,
            motion.date,
            &replies,
            &roster,
            &policy,
            Utc::now(),
        )?;

        info!(
            ballots = tally.ballots.len(),
            rejected = tally.rejected.len(),
            outcome = ?tally.outcome,
            open = tally.open,
            "counted"
        );
        Ok(Count {
            list,
            uid: thread.uid().to_string(),
            name: thread.name().to_string(),
            url: thread.url(),
            tally,
        })
    }
}

impl Voting {
    async fn roster(&self) -> Result<Roster, VotingError> {
        let path = self
            .config
            .roster_path
            .as_ref()
            .ok_or(VotingError::NoRoster)?;
        let json = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| VotingError::Roster(error.to_string()))?;
        Roster::parse(&json).map_err(|error| VotingError::Roster(error.to_string()))
    }

    async fn thread(
        &self,
        list: MailingListEnum,
        thread: &str,
        requested_by: &str,
    ) -> Result<Option<Thread>, VotingError> {
        let requested_by = requested_by.to_string();
        let thread = match Reference::parse(thread) {
            Reference::Uid(uid) => {
                let fetch = FetchThread {
                    list,
                    uid,
                    requested_by,
                };
                self.reader.ask(fetch).await?
            }
            Reference::MessageId(message_id) => {
                let find = FindThread {
                    list,
                    message_id,
                    requested_by,
                };
                self.reader.ask(find).await?
            }
        };
        Ok(thread)
    }
}

/// The ways a user can point at a thread
#[derive(Debug, PartialEq, Eq)]
enum Reference {
    Uid(String),
    /// Of any mail in the thread
    MessageId(String),
}

impl Reference {
    fn parse(input: &str) -> Self {
        let input = input.trim();

        // https://lists.example.org/<list>/<message-id>/T/#u
        if let Some((_, path)) = input.split_once("://") {
            let message_id = path.split(['/', '#', '?']).nth(2).unwrap_or_default();
            return Self::MessageId(percent_decode(message_id));
        }

        let input = input.trim_start_matches('<').trim_end_matches('>');
        if input.contains('@') {
            Self::MessageId(input.to_string())
        } else {
            Self::Uid(input.to_lowercase())
        }
    }
}

/// Undo the escaping public-inbox applies to Message-IDs in its URLs
fn percent_decode(escaped: &str) -> String {
    let mut bytes = Vec::with_capacity(escaped.len());
    let mut rest = escaped.as_bytes();
    while let [byte, tail @ ..] = rest {
        let hex = tail
            .get(..2)
            .filter(|_| *byte == b'%')
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match hex {
            Some(decoded) => {
                bytes.push(decoded);
                rest = &tail[2..];
            }
            None => {
                bytes.push(*byte);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_id(id: &str) -> Reference {
        Reference::MessageId(id.to_string())
    }

    #[test]
    fn a_link_points_at_its_mail() {
        for link in [
            "https://lists.opengamingcollective.org/governance/2ba6@gmail.com/T/#u",
            "https://lists.opengamingcollective.org/governance/2ba6@gmail.com/",
            "https://lists.opengamingcollective.org/governance/2ba6@gmail.com",
            " https://lists.opengamingcollective.org/governance/2ba6@gmail.com/t.mbox.gz ",
        ] {
            assert_eq!(
                Reference::parse(link),
                message_id("2ba6@gmail.com"),
                "{link}"
            );
        }
    }

    #[test]
    fn escaped_links_are_decoded() {
        assert_eq!(
            Reference::parse("https://l.example.org/list/a%2Fb%20c@d/T/"),
            message_id("a/b c@d")
        );
        assert_eq!(percent_decode("100%@d%2"), "100%@d%2");
    }

    #[test]
    fn a_message_id_points_at_its_mail() {
        assert_eq!(Reference::parse("<a.b@c>"), message_id("a.b@c"));
        assert_eq!(Reference::parse("a.b@c"), message_id("a.b@c"));
    }

    #[test]
    fn anything_else_is_a_uid() {
        assert_eq!(
            Reference::parse(" 0A1b2c3d4e5f "),
            Reference::Uid("0a1b2c3d4e5f".to_string())
        );
        // The reader reports that it isn't a thread
        assert_eq!(Reference::parse("https://example.org"), message_id(""));
    }
}
