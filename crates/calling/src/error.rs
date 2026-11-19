//! Errors for the calling crate.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Proto(#[from] teams_proto::Error),
    #[error("{op}: HTTP {status}: {body}")]
    Status {
        op: &'static str,
        status: u16,
        body: String,
    },
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("media: {0}")]
    Media(String),
    #[error("audio: {0}")]
    Audio(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
