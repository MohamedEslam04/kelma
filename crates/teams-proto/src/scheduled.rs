//! Scheduled send ("Schedule message" in the send button's menu): the chat service keeps the
//! message as a `ScheduledDraft` and sends it at `sendAt`.
//!
//! Endpoints from rusty-teams `drafts.rs` (`docs/research/open-source.md` §3): `POST
//! /v1/users/ME/drafts` creates one (the answer has no id, it is found again by its client
//! message id), `GET /v1/users/ME/drafts` lists them, `DELETE /v1/users/ME/drafts/<id>` cancels.

use crate::chatsvc::send_body;
use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::{Value, json};

/// A message waiting to be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledMessage {
    pub id: String,
    pub conversation_id: String,
    pub send_at_ms: i64,
    pub html: String,
    pub client_message_id: String,
}

/// The body that schedules `html` for `send_at_ms` in `conversation_id`.
pub fn schedule_body(
    conversation_id: &str,
    html: &str,
    display_name: &str,
    client_message_id: &str,
    send_at_ms: i64,
) -> Value {
    let mut message = send_body(html, display_name, client_message_id, None);
    message["properties"] = json!({});
    json!({
        "draftDetails": { "sendAt": send_at_ms.to_string() },
        "draftType": "ScheduledDraft",
        "innerThreadId": conversation_id,
        "message": message,
    })
}

fn num(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// The scheduled messages in a drafts list (other draft kinds and cancelled ones are left out).
///
/// The list (`view=msnp24Equivalent`) gives the message fields at the top level of each draft
/// (`content`, `clientmessageid`, as rusty-teams reads them); the nested `message` object of the
/// create body is read as a fallback.
pub fn parse_scheduled(v: &Value) -> Vec<ScheduledMessage> {
    let list = v
        .get("drafts")
        .or_else(|| v.get("value"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    list.iter()
        .filter(|d| d.get("draftType").and_then(Value::as_str) == Some("ScheduledDraft"))
        .filter(|d| {
            d.pointer("/properties/deletetime")
                .is_none_or(Value::is_null)
        })
        .filter_map(|d| {
            let s = |p: &str| {
                d.pointer(p)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            };
            let field =
                |name: &str| s(&format!("/{name}")).or_else(|| s(&format!("/message/{name}")));
            Some(ScheduledMessage {
                id: s("/id")?,
                conversation_id: s("/innerThreadId").unwrap_or_default(),
                send_at_ms: num(d.pointer("/draftDetails/sendAt")).unwrap_or_default(),
                html: field("content").unwrap_or_default(),
                // The bubbles are addressed by it, so a draft without one goes by its id.
                client_message_id: field("clientmessageid").or_else(|| s("/id"))?,
            })
        })
        .collect()
}

impl Client {
    async fn drafts_url(&self) -> String {
        format!("{}/v1/users/ME/drafts", self.chat_service().await)
    }

    /// Schedules `html` in `conversation_id` for `send_at_ms`. Returns its client message id.
    pub async fn schedule_message(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        send_at_ms: i64,
    ) -> Result<String> {
        let cmid = crate::time::now_ms().to_string();
        let body = schedule_body(conversation_id, html, display_name, &cmid, send_at_ms);
        let url = self.drafts_url().await;
        self.json("schedule message", Method::POST, &url, Some(&body))
            .await?;
        Ok(cmid)
    }

    /// The messages waiting to be sent.
    pub async fn scheduled_messages(&self) -> Result<Vec<ScheduledMessage>> {
        let url = format!(
            "{}?view=msnp24Equivalent&pageSize=200&startTime=1",
            self.drafts_url().await
        );
        let v = self
            .json("scheduled messages", Method::GET, &url, None)
            .await?;
        Ok(parse_scheduled(&v))
    }

    /// Cancels a scheduled message.
    pub async fn cancel_scheduled(&self, id: &str) -> Result<()> {
        let url = format!(
            "{}/{}",
            self.drafts_url().await,
            crate::chatsvc::encode_conversation(id)
        );
        self.json("cancel scheduled", Method::DELETE, &url, None)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheduled_draft_carries_the_message_and_the_time() {
        let b = schedule_body("48:notes", "<p>hi</p>", "Me", "123", 1_800_000_000_000);
        assert_eq!(b["draftType"], "ScheduledDraft");
        assert_eq!(b["innerThreadId"], "48:notes");
        assert_eq!(b["draftDetails"]["sendAt"], "1800000000000");
        assert_eq!(b["message"]["content"], "<p>hi</p>");
        assert_eq!(b["message"]["clientmessageid"], "123");
        assert_eq!(b["message"]["messagetype"], "RichText/Html");
    }

    #[test]
    fn only_scheduled_drafts_are_listed() {
        let v = json!({"drafts": [
            {"id": "d1", "draftType": "ScheduledDraft", "innerThreadId": "19:a@thread.v2",
             "draftDetails": {"sendAt": "1800000000000"},
             "message": {"content": "<p>later</p>", "clientmessageid": "c1"}},
            {"id": "d2", "draftType": "Draft", "innerThreadId": "19:b@thread.v2"}
        ]});
        let s = parse_scheduled(&v);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].id, "d1");
        assert_eq!(s[0].send_at_ms, 1_800_000_000_000);
        assert_eq!(s[0].html, "<p>later</p>");
    }

    #[test]
    fn the_listed_drafts_carry_the_message_fields_at_the_top_level() {
        // The shape of `GET /v1/users/ME/drafts?view=msnp24Equivalent` (rusty-teams drafts.rs).
        let v = json!({"drafts": [
            {"id": "d1", "draftType": "ScheduledDraft", "innerThreadId": "19:a@thread.v2",
             "draftDetails": {"sendAt": "1800000000000"},
             "content": "<p>this is a test</p>", "clientmessageid": "c1",
             "messagetype": "RichText/Html", "properties": {}},
            {"id": "d2", "draftType": "ScheduledDraft", "innerThreadId": "19:a@thread.v2",
             "draftDetails": {"sendAt": "1800000000000"},
             "content": "<p>cancelled</p>", "clientmessageid": "c2",
             "properties": {"deletetime": "1799999999999"}}
        ]});
        let s = parse_scheduled(&v);
        assert_eq!(s.len(), 1, "the cancelled one is left out");
        assert_eq!(s[0].html, "<p>this is a test</p>");
        assert_eq!(s[0].client_message_id, "c1");
    }
}
