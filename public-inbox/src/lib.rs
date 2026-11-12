//! Mirrors public-inbox mailing lists over git, parsing threads and raw messages.
//! TODO: repost mails to the discord
mod epoch;
mod errors;
mod mailing_list;
mod manifest;
mod members;
mod message;
mod mirror;
mod seen;
mod thread;

pub use errors::{MailingListError, Result};
pub use mail_parser::Message;
pub use mailing_list::MailingList;
pub use members::{Member, OrgList, parse_members_json};
pub use message::Mail;
pub use thread::Thread;
