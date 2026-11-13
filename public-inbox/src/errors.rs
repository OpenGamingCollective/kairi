use std::io;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MailingListError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("git: {0}")]
    Git(#[from] gix::Error),

    #[error("background task: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("failed to parse mail")]
    MailParseFailed,

    #[error("epoch {0} is not mirrored")]
    EpochNotFound(u32),
}

pub type Result<T, E = MailingListError> = std::result::Result<T, E>;
