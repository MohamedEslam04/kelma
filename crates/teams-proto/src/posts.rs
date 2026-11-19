//! Channel posts (plan 8.4): announcement posts and following a thread.
//!
//! An announcement is a normal post whose `properties.title` is the headline and whose
//! `properties.cards` holds one banner card (`messaging-announcementBanner`) with the color theme.
//! Following a thread is a flag on the thread's reply-chain metadata:
//! `PUT v1/users/ME/conversations/<channel>/rcmetadata/<root id>` `{"f": 1}` (the same
//! resource the Save flag lives in, see [`crate::actions`]); the flag is the number 1 or 0, a
//! JSON `true` is refused with `400`. Both shapes come from the Teams web
//! client's public JavaScript and are checked live against the sandbox channel, see
//! `docs/protocol.md`, "Channel posts".

use crate::chat_card::ANNOUNCEMENT_BANNER;
use crate::chatsvc::{encode_conversation, send_body};
use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::{Value, json};

/// The color themes of an announcement banner.
pub const BANNER_THEMES: [&str; 5] = ["blue", "green", "yellow", "red", "purple"];

/// The `properties` of an announcement post.
pub fn announcement_properties(subject: &str, headline: &str, theme: &str) -> Value {
    let theme = if BANNER_THEMES.contains(&theme) {
        theme
    } else {
        "blue"
    };
    let card = json!({
        "contentType": ANNOUNCEMENT_BANNER,
        "content": {"imageData": {"colorTheme": theme}},
    });
    let mut props = json!({
        "title": headline,
        "cards": json!([card]).to_string(),
    });
    if !subject.is_empty() {
        props["subject"] = Value::from(subject);
    }
    props
}

/// Request body of an announcement post.
pub fn announcement_body(
    html: &str,
    display_name: &str,
    client_message_id: &str,
    subject: &str,
    headline: &str,
    theme: &str,
) -> Value {
    let mut b = send_body(html, display_name, client_message_id, None);
    b["properties"] = announcement_properties(subject, headline, theme);
    b
}

/// Body of the follow flag.
pub fn follow_body(follow: bool) -> Value {
    json!({ "f": u8::from(follow) })
}

impl Client {
    /// Posts an announcement in a channel.
    pub async fn send_announcement(
        &self,
        channel_id: &str,
        html: &str,
        display_name: &str,
        subject: &str,
        headline: &str,
        theme: &str,
    ) -> Result<String> {
        let cmid = crate::time::now_ms().to_string();
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages",
            self.chat_service().await,
            encode_conversation(channel_id)
        );
        let body = announcement_body(html, display_name, &cmid, subject, headline, theme);
        self.json("announcement", Method::POST, &url, Some(&body))
            .await?;
        Ok(cmid)
    }

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
    use crate::chat_card::{Band, Kind, rows_for_attachments};

    #[test]
    fn an_announcement_carries_a_headline_and_a_banner_card() {
        let b = announcement_body("<p>hi</p>", "Me", "7", "Release", "Big news", "green");
        assert_eq!(b["content"], "<p>hi</p>");
        assert_eq!(b["properties"]["title"], "Big news");
        assert_eq!(b["properties"]["subject"], "Release");
        // `cards` is a JSON string, like the other message properties.
        let cards: Value =
            serde_json::from_str(b["properties"]["cards"].as_str().unwrap()).unwrap();
        let rows = rows_for_attachments(&cards, "Big news");
        assert_eq!(rows[0].kind, Kind::Banner);
        assert_eq!(rows[0].band, Band::Good);
        assert_eq!(rows[0].text, "Big news");
    }

    #[test]
    fn an_unknown_theme_falls_back_to_blue_and_no_subject_is_left_out() {
        let p = announcement_properties("", "H", "chartreuse");
        assert!(p.get("subject").is_none());
        assert!(p["cards"].as_str().unwrap().contains("\"blue\""));
    }

    #[test]
    fn follow_flag_body() {
        assert_eq!(follow_body(true), json!({"f": 1}));
        assert_eq!(follow_body(false), json!({"f": 0}));
    }
}
