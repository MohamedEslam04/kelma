//! Error type for the protocol crate.

/// Errors returned by the protocol layer.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("websocket: {0}")]
    Ws(#[from] Box<tokio_tungstenite::tungstenite::Error>),
    #[error("{context}: HTTP {status}: {body}")]
    Status {
        context: &'static str,
        status: u16,
        body: String,
    },
    #[error("auth: {0}")]
    Auth(String),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("device code expired before sign-in finished")]
    DeviceCodeExpired,
    #[error("not signed in")]
    NotSignedIn,
}

/// Result alias for the protocol crate.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Builds a status error, truncating the body so logs stay small.
    pub fn status(context: &'static str, status: u16, body: &str) -> Self {
        let body: String = body.chars().take(400).collect();
        Error::Status {
            context,
            status,
            body,
        }
    }
}
