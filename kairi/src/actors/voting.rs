//! Actor that counts the votes on a mailing list thread

use jiff::Timestamp;
use kairi_config::VotingConfig;
use kameo::{error::SendError, prelude::*};
use public_inbox::{MailingListError, Thread};
use thiserror::Error;
use tracing::{info, instrument};
use voting::{MotionError, Reply, Roster, Tally};

use crate::{
    actors::{
        NotRunning,
        public_inbox_reader::{FetchMessages, FetchThread, FindThread},
        reader,
    },
    types::MailingListEnum,
};

pub struct Voting {
    config: VotingConfig,
}

#[derive(Debug, Clone)]
pub struct CountVotes {
    pub list: MailingListEnum,
    pub thread: String,
    pub requested_by: String,
}

#[derive(Debug, Clone)]
pub struct Count {
    pub list: MailingListEnum,
    pub thread: Thread,
    pub roster: Roster,
    pub tally: Tally,
}

#[derive(Debug, Error)]
pub enum VotingError {
    #[error(transparent)]
    NotRunning(#[from] NotRunning),

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
    type Args = VotingConfig;
    type Error = anyhow::Error;

    fn name() -> &'static str {
        "voting"
    }

    async fn on_start(config: Self::Args, actor_ref: ActorRef<Self>) -> Result<Self, Self::Error> {
        actor_ref.register(<Self as Actor>::name())?;
        Ok(Self { config })
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

        let thread = self.thread(list, &thread, &requested_by).await?;
        let messages = reader()?
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

        let tally = voting::tally(
            &motion.body,
            motion.date,
            &replies,
            &roster,
            &self.config.policy,
            Timestamp::now(),
        )?;

        info!(
            ballots = tally.cast(),
            rejected = tally.rejected.len(),
            outcome = ?tally.outcome,
            open = tally.open,
            "counted"
        );
        Ok(Count {
            list,
            thread,
            roster,
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
    ) -> Result<Thread, VotingError> {
        let requested_by = requested_by.to_string();
        let thread = match Reference::parse(thread) {
            Reference::Uid(uid) => {
                reader()?
                    .ask(FetchThread {
                        list,
                        uid,
                        requested_by,
                    })
                    .await?
            }
            Reference::MessageId(message_id) => {
                reader()?
                    .ask(FindThread {
                        list,
                        message_id,
                        requested_by,
                    })
                    .await?
            }
        };
        thread.ok_or(VotingError::NoSuchThread)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Reference {
    Uid(String),
    MessageId(String),
}

impl Reference {
    fn parse(input: &str) -> Self {
        let input = input.trim();

        // https://lists.example.org/<list>/<message-id>/T/#u
        if let Some((_, path)) = input.split_once("://") {
            let message_id = path.split('/').nth(2).unwrap_or_default();
            let message_id = percent_encoding::percent_decode_str(message_id).decode_utf8_lossy();
            return Self::MessageId(message_id.into_owned());
        }

        let input = input.trim_start_matches('<').trim_end_matches('>');
        if input.contains('@') {
            Self::MessageId(input.to_string())
        } else {
            Self::Uid(input.to_lowercase())
        }
    }
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
        assert_eq!(
            Reference::parse("https://l.example.org/list/100%@d%2/"),
            message_id("100%@d%2")
        );
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
