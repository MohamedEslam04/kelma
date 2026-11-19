//! Channel posts (plan 8.4): following a thread.
//!
//! Following a thread is a flag on the thread's reply-chain metadata:
//! `PUT v1/users/ME/conversations/<channel>/rcmetadata/<root id>` `{"f": 1}` (the same resource
//! the Save flag lives in, see [`crate::actions`]); the flag is the number 1 or 0, a JSON `true` is
//! refused with `400`. The shape comes from the Teams web client's public JavaScript and was
//! checked live against the sandbox channel, see `docs/protocol.md`, "Channel posts".
//!
//! Announcement posts are sent with [`crate::delivery`].

use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::{Value, json};

/// Body of the follow flag.
pub fn follow_body(follow: bool) -> Value {
    json!({ "f": u8::from(follow) })
}

impl Client {
    /// Follows (`true`) or unfollows a channel thread, so replies to it notify.
    pub async fn set_thread_followed(
        &self,
        channel_id: &str,
        root_id: &str,
        follow: bool,
    ) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/rcmetadata/{}",
            self.chat_service().await,
            encode_conversation(channel_id),
            encode_conversation(root_id)
        );
        self.json(
            "follow thread",
            Method::PUT,
            &url,
            Some(&follow_body(follow)),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_flag_body() {
        assert_eq!(follow_body(true), json!({"f": 1}));
        assert_eq!(follow_body(false), json!({"f": 0}));
    }
}
