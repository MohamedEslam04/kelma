//! Chat service: conversations, message history, sending, reactions, edits, read markers.

use crate::client::Client;
use crate::error::{Error, Result};
use crate::model::{Conversation, Message, conversation_from_json, message_from_json};
use crate::time::now_ms;
use reqwest::Method;
use serde_json::{Value, json};

const TARGET_TYPES: &str = "Passport|Skype|Lync|Thread|PSTN|Agent";

/// Percent-encodes a conversation id for use in a URL path, keeping a `;messageid=` thread suffix.
pub fn encode_conversation(id: &str) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    match id.split_once(";messageid=") {
        Some((c, m)) => format!("{};messageid={}", enc(c), enc(m)),
        None => enc(id),
    }
}

/// Request body for sending a message.
pub fn send_body(
    html: &str,
    display_name: &str,
    client_message_id: &str,
    subject: Option<&str>,
) -> Value {
    let mut b = json!({
        "clientmessageid": client_message_id,
        "content": html,
        "messagetype": "RichText/Html",
        "contenttype": "text",
        "imdisplayname": display_name,
    });
    if let Some(s) = subject.filter(|s| !s.is_empty()) {
        b["properties"] = json!({ "subject": s });
    }
    b
}

/// `properties.qtdMsgs` of a "Reply with quote" message. It is a JSON *string* naming the quoted
/// message, which lets Teams link the quote block to its original.
pub fn quoted_messages_property(message_id: &str, sender_mri: &str, time_ms: i64) -> String {
    let id: Value = message_id
        .parse::<i64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::from(message_id));
    json!([{ "messageId": id, "sender": sender_mri, "time": time_ms }]).to_string()
}

/// Value for the consumption horizon property.
pub fn horizon_value(message_id: &str, compose_ms: i64, client_message_id: &str) -> String {
    format!("{message_id};{compose_ms};{client_message_id}")
}

/// Pages of history returned by [`Client::messages`].
pub struct HistoryPage {
    /// Messages in ascending time order.
    pub messages: Vec<Message>,
    /// True when an older page probably exists.
    pub more: bool,
}

impl Client {
    async fn conv_url(&self, id: &str) -> String {
        format!(
            "{}/v1/users/ME/conversations/{}",
            self.chat_service().await,
            encode_conversation(id)
        )
    }

    /// Lists conversations active since `since_ms` (0 = everything), newest first.
    pub async fn conversations(&self, since_ms: i64, page_size: u32) -> Result<Vec<Conversation>> {
        let url = format!(
            "{}/v1/users/ME/conversations?view=msnp24Equivalent&pageSize={page_size}&startTime={}&targetType={TARGET_TYPES}",
            self.chat_service().await,
            since_ms.max(1)
        );
        let v = self.json("conversations", Method::GET, &url, None).await?;
        Ok(v.get("conversations")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(conversation_from_json)
                    .filter(|c| !c.id.starts_with("48:") || c.id == "48:notes")
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Fetches one page of history ending before `before_ms` (or the newest page).
    pub async fn messages(
        &self,
        conversation_id: &str,
        before_ms: Option<i64>,
        page_size: u32,
    ) -> Result<HistoryPage> {
        let mut url = format!(
            "{}/messages?view=msnp24Equivalent|supportsMessageProperties&pageSize={page_size}&startTime=1",
            self.conv_url(conversation_id).await
        );
        if let Some(b) = before_ms {
            url.push_str(&format!("&endTime={}", b - 1));
        }
        let v = self
            .json("messages", Method::GET, &url, None)
            .await
            .map_err(Error::into_not_member)?;
        let raw = v
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let more = raw.len() as u32 >= page_size;
        let mut messages: Vec<Message> = raw
            .iter()
            .filter_map(|m| message_from_json(m, conversation_id))
            .collect();
        messages.sort_by_key(|m| m.compose_ms);
        Ok(HistoryPage { messages, more })
    }

    /// Sends an HTML message. For a channel reply pass `"<channel id>;messageid=<root id>"`.
    /// Returns the client message id used (to recognize our own echo).
    pub async fn send_message(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        subject: Option<&str>,
    ) -> Result<String> {
        let cmid = now_ms().to_string();
        self.send_message_as(conversation_id, html, display_name, subject, &cmid)
            .await?;
        Ok(cmid)
    }

    /// Like [`Client::send_message`] with a caller-chosen client message id. Sending the same id
    /// again (a retry after a lost response) does not post a second message.
    pub async fn send_message_as(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        subject: Option<&str>,
        client_message_id: &str,
    ) -> Result<()> {
        let url = format!("{}/messages", self.conv_url(conversation_id).await);
        self.json(
            "send",
            Method::POST,
            &url,
            Some(&send_body(html, display_name, client_message_id, subject)),
        )
        .await
        .map(|_| ())
    }

    /// Sends a "reply with quote": `html` holds the quote block and the reply, and `qtd_msgs` is
    /// [`quoted_messages_property`].
    pub async fn send_quote_reply(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        qtd_msgs: &str,
    ) -> Result<String> {
        let cmid = now_ms().to_string();
        self.send_quote_reply_as(conversation_id, html, display_name, qtd_msgs, &cmid)
            .await?;
        Ok(cmid)
    }

    /// Like [`Client::send_quote_reply`] with a caller-chosen client message id.
    pub async fn send_quote_reply_as(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        qtd_msgs: &str,
        client_message_id: &str,
    ) -> Result<()> {
        let url = format!("{}/messages", self.conv_url(conversation_id).await);
        let mut body = send_body(html, display_name, client_message_id, None);
        body["properties"] = json!({ "qtdMsgs": qtd_msgs });
        self.json("send_quote_reply", Method::POST, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Sends a message carrying an uploaded file (`files` is a [`crate::files::files_property`] value).
    pub async fn send_file_message(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        files: &str,
    ) -> Result<String> {
        let cmid = now_ms().to_string();
        self.send_file_message_as(conversation_id, html, display_name, files, &cmid)
            .await?;
        Ok(cmid)
    }

    /// Like [`Client::send_file_message`] with a caller-chosen client message id.
    pub async fn send_file_message_as(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        files: &str,
        client_message_id: &str,
    ) -> Result<()> {
        let url = format!("{}/messages", self.conv_url(conversation_id).await);
        let mut body = send_body(html, display_name, client_message_id, None);
        body["properties"] = json!({ "files": files });
        self.json("send_file", Method::POST, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Replaces the content of one of our messages.
    pub async fn edit_message(
        &self,
        conversation_id: &str,
        message_id: &str,
        html: &str,
        display_name: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/messages/{message_id}",
            self.conv_url(conversation_id).await
        );
        let body = json!({"content": html, "messagetype": "RichText/Html", "contenttype": "text", "imdisplayname": display_name});
        self.json("edit", Method::PUT, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Soft-deletes one of our messages.
    pub async fn delete_message(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        let url = format!(
            "{}/messages/{message_id}?behavior=softDelete",
            self.conv_url(conversation_id).await
        );
        self.json("delete", Method::DELETE, &url, None)
            .await
            .map(|_| ())
    }

    /// Adds a reaction (`like`, `heart`, `laugh`, `surprised`, `sad`, `angry`, …).
    pub async fn add_reaction(
        &self,
        conversation_id: &str,
        message_id: &str,
        key: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/messages/{message_id}/properties?name=emotions",
            self.conv_url(conversation_id).await
        );
        let body = json!({"emotions": {"key": key, "value": now_ms()}});
        self.json("react", Method::PUT, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Removes our reaction.
    pub async fn remove_reaction(
        &self,
        conversation_id: &str,
        message_id: &str,
        key: &str,
    ) -> Result<()> {
        let url = format!(
            "{}/messages/{message_id}/properties?name=emotions",
            self.conv_url(conversation_id).await
        );
        let body = json!({"emotions": {"key": key}});
        self.json("unreact", Method::DELETE, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Marks everything up to `message` as read.
    pub async fn mark_read(&self, conversation_id: &str, message: &Message) -> Result<()> {
        let url = format!(
            "{}/properties?name=consumptionhorizon",
            self.conv_url(conversation_id).await
        );
        let cmid = message
            .client_message_id
            .clone()
            .unwrap_or_else(|| message.id.clone());
        let body =
            json!({"consumptionhorizon": horizon_value(&message.id, message.compose_ms, &cmid)});
        self.json("mark_read", Method::PUT, &url, Some(&body))
            .await?;
        // A manual "Mark as unread" survives a new read horizon; clear it too.
        if let Err(e) = self.clear_unread_mark(conversation_id).await {
            tracing::debug!(%e, "clearing the unread bookmark failed");
        }
        Ok(())
    }

    /// Sends a typing indicator.
    pub async fn send_typing(&self, conversation_id: &str) -> Result<()> {
        let url = format!("{}/messages", self.conv_url(conversation_id).await);
        let body = json!({"content": "", "contenttype": "Application/Message", "messagetype": "Control/Typing"});
        self.json("typing", Method::POST, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Our own MRI, e.g. `8:orgid:<guid>`.
    pub async fn self_mri(&self) -> Result<String> {
        let url = format!("{}/v1/users/ME/properties", self.chat_service().await);
        let v = self.json("self", Method::GET, &url, None).await?;
        let name = v
            .get("skypeName")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("no skypeName in self properties".into()))?;
        Ok(if name.starts_with("8:") {
            name.to_string()
        } else {
            format!("8:{name}")
        })
    }

    /// Members (MRIs) and topic of a group thread.
    pub async fn thread_members(
        &self,
        conversation_id: &str,
    ) -> Result<(Vec<String>, Option<String>)> {
        let url = format!(
            "{}/v1/threads/{}?view=msnp24Equivalent",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let v = self.json("thread", Method::GET, &url, None).await?;
        let members = v
            .get("members")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let topic = v
            .pointer("/properties/topic")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        Ok((members, topic))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_ids() {
        assert_eq!(
            encode_conversation("19:abc@thread.v2"),
            "19%3Aabc%40thread.v2"
        );
        assert_eq!(
            encode_conversation("19:c@thread.tacv2;messageid=123"),
            "19%3Ac%40thread.tacv2;messageid=123"
        );
        assert_eq!(encode_conversation("48:notes"), "48%3Anotes");
    }

    #[test]
    fn send_body_shape() {
        let b = send_body("<p>hi</p>", "Me", "42", None);
        assert_eq!(b["messagetype"], "RichText/Html");
        assert_eq!(b["contenttype"], "text");
        assert_eq!(b["clientmessageid"], "42");
        assert!(b.get("properties").is_none());
        let b = send_body("<p>hi</p>", "Me", "42", Some("Release notes"));
        assert_eq!(b["properties"]["subject"], "Release notes");
    }

    #[test]
    fn quoted_message_property_is_a_json_string() {
        let v = quoted_messages_property("1731234567890", "8:orgid:abc", 1731234567890);
        let parsed: Value = serde_json::from_str(&v).unwrap();
        assert_eq!(
            parsed,
            json!([{"messageId": 1731234567890_i64, "sender": "8:orgid:abc", "time": 1731234567890_i64}])
        );
        let odd = quoted_messages_property("abc", "8:x", 1);
        assert!(odd.contains(r#""messageId":"abc""#));
    }

    #[test]
    fn horizon_format() {
        assert_eq!(horizon_value("1795", 1795, "99"), "1795;1795;99");
    }
}
