//! This library handles scraping a public-inbox instance and its mailing lists,
//! parsing threads and raw messages.
//! TODO: repost mails to the discord
mod errors;
mod mailing_list;
mod members;
mod message;
mod thread;

pub use errors::{MailingListError, Result};
pub use mailing_list::MailingList;
pub use members::{Member, OrgList, parse_members_json};
pub use message::{author_of, fetch_messages, mail_dt_to_chrono, mbox_url, sender_of};
pub use thread::Thread;
