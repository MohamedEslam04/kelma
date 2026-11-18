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

impl Error {
    /// True when the request may succeed later without changes: no connection, a timeout, a
    /// dropped socket or a temporary server answer. False for rejections such as 400 or 403.
    pub fn is_transient(&self) -> bool {
        match self {
            Error::Http(e) => e.is_connect() || e.is_timeout() || e.is_request(),
            Error::Ws(_) => true,
            Error::Status { status, .. } => matches!(status, 408 | 425 | 429 | 502 | 503 | 504),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_status_codes_are_transient_and_rejections_are_not() {
        for s in [408, 429, 502, 503, 504] {
            assert!(Error::status("send", s, "").is_transient(), "{s}");
        }
        for s in [400, 401, 403, 404, 409, 500] {
            assert!(!Error::status("send", s, "").is_transient(), "{s}");
        }
        assert!(!Error::Protocol("x".into()).is_transient());
        assert!(!Error::Auth("x".into()).is_transient());
        assert!(!Error::NotSignedIn.is_transient());
    }

    #[test]
    fn socket_errors_are_transient() {
        let e = tokio_tungstenite::tungstenite::Error::ConnectionClosed;
        assert!(Error::Ws(Box::new(e)).is_transient());
    }

    #[tokio::test]
    async fn refused_connection_is_transient() {
        // Nothing listens on port 1.
        let err = reqwest::Client::new()
            .get("http://127.0.0.1:1/")
            .send()
            .await
            .unwrap_err();
        assert!(Error::Http(err).is_transient());
    }
}
