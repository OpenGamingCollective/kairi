use std::io;

use reqwest::StatusCode;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MailingListError {
    #[error("IO error {0}")]
    Disconnect(#[from] io::Error),

    #[error("failed to parse url {0}")]
    UrlParseError(#[from] url::ParseError),

    #[error("reqwest error {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("serde_json error {0}")]
    SerdeJsonError(#[from] serde_json::Error),

    #[error("failed to parse mail")]
    MailParseFailed,

    #[error("thread contained no mail")]
    ThreadEmpty,

    #[error("could not find the first message")]
    NoFirstMessageFound,

    #[error("could not get the first message date")]
    NoFirstMessageDate,

    #[error("could not convert the mail dt to chrono")]
    DtToChronoError,

    #[error("{0} return code is not OK: {1}")]
    UrlStatusCodeError(String, StatusCode),

    #[error("uid cannot be empty")]
    ThreadUidEmpty,

    #[error("thread with uid {0} not found")]
    ThreadNotFound(String),
}

pub type Result<T, E = MailingListError> = std::result::Result<T, E>;
