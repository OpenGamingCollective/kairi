//! Mirrors public-inbox mailing lists over git and indexes them, reads them locally
mod epoch;
mod errors;
mod index;
mod manifest;
mod message;
mod mirror;
mod reader;
mod thread;

pub use errors::{MailingListError, Result};
pub use mail_parser::Message;
pub use message::Mail;
pub use mirror::{Mirror, reindex};
pub use reader::{List, Reader};
pub use thread::{Thread, ThreadSummary};

