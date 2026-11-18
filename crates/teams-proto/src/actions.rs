//! Message and conversation actions behind the Teams message "…" menu and chat list menu:
//! forward, save, pin, mark as unread, mark all as read, mute and hide.
//!
//! Endpoints are documented in `docs/protocol.md` ("Message actions"). Sources: rusty-teams
//! (MIT) `forward.rs`, `saved.rs`, `pins/pinbar.rs`, `read_horizon.rs`; Jezza/ic3 (verified hide
//! and unread bookmark).

use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::error::Result;
use crate::time::now_ms;
use reqwest::Method;
use serde_json::{Value, json};

/// CSA base used for pinbar requests.
const CSA_V1: &str = "https://teams.microsoft.com/api/csa/api/v1";

/// Result of forwarding to one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forwarded {
    pub thread_id: String,
    pub status: u16,
    pub message_id: Option<String>,
}

/// A message the user saved ("Save this message"), from the `48:saved` virtual conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedMessage {
    pub conversation_id: String,
    pub message_id: String,
    /// Root post id for channel replies; equals `message_id` elsewhere.
    pub root_id: String,
    pub sender_name: String,
    pub preview: String,
    pub topic: String,
    pub saved_ms: i64,
}

/// A message pinned for everyone in a chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedMessage {
    pub message_id: String,
    /// Folder the pin lives in, empty at top level.
    pub folder_id: String,
    pub pinned_at: String,
}

/// Body for `POST …/messages/forward`. Teams accepts at most five message ids per request.
pub fn forward_body(
    message_ids: &[String],
    targets: &[String],
    note_html: Option<&str>,
    display_name: &str,
    client_message_id: &str,
) -> Value {
    json!({
        "targetThreadIds": targets,
        "messageIds": message_ids.iter().take(5).collect::<Vec<_>>(),
        "additionalMessage": {
            "clientmessageid": client_message_id,
            "content": note_html.unwrap_or(""),
            "messagetype": "RichText/Html",
            "contenttype": "text",
            "imdisplayname": display_name,
            "properties": {"importance": "", "subject": ""},
        },
        "templateId": "basic_forward_message_template",
    })
}

/// Parses the forward answer `[{statusCode, threadId, messageId}]`.
pub fn parse_forwarded(v: &Value) -> Vec<Forwarded> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|r| Forwarded {
                    thread_id: str_of(r, "threadId"),
                    status: r.get("statusCode").and_then(Value::as_u64).unwrap_or(0) as u16,
                    message_id: r.get("messageId").and_then(|m| {
                        m.as_str()
                            .map(str::to_owned)
                            .or(m.as_u64().map(|n| n.to_string()))
                    }),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k)
        .map(|x| match x {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

/// Parses `48:saved` messages. `properties.activity` is an object or a JSON string.
pub fn parse_saved(v: &Value) -> Vec<SavedMessage> {
    let Some(msgs) = v.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };
    msgs.iter()
        .filter_map(|m| {
            let act = m.pointer("/properties/activity")?;
            let act: Value = match act {
                Value::String(s) => serde_json::from_str(s).ok()?,
                other => other.clone(),
            };
            if act.get("activityType").and_then(Value::as_str) != Some("savedMessage") {
                return None;
            }
            let message_id = str_of(&act, "sourceMessageId");
            let root = str_of(&act, "sourceReplyChainId");
            Some(SavedMessage {
                conversation_id: str_of(&act, "sourceThreadId"),
                root_id: if root.is_empty() || root == "0" {
                    message_id.clone()
                } else {
                    root
                },
                message_id,
                sender_name: str_of(&act, "sourceUserImDisplayName"),
                preview: str_of(&act, "messagePreview"),
                topic: str_of(&act, "sourceThreadTopic"),
                saved_ms: crate::time::parse_iso_ms(&str_of(&act, "activityTimestamp"))
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Body for adding or removing a pinbar message.
pub fn pinbar_body(chat_id: &str, message_id: &str, folder_id: &str, add: bool) -> Value {
    json!({
        "pinbarId": chat_id,
        "pinbarItemType": "pinmsg",
        "pinbarItemId": message_id,
        "pinbarParentItemId": folder_id,
        "pinbarOperation": if add { "add" } else { "remove" },
        "pinbarItemPosition": 0,
        "pinnedMessageProperties": {"messageId": message_id, "rootMessageId": message_id},
    })
}

/// Parses a pinbar, flattening folders.
pub fn parse_pinbar(v: &Value) -> Vec<PinnedMessage> {
    fn walk(items: &Value, folder: &str, out: &mut Vec<PinnedMessage>) {
        for it in items.as_array().into_iter().flatten() {
            match it.get("type").and_then(Value::as_str) {
                Some("pinmsg") => out.push(PinnedMessage {
                    message_id: it
                        .pointer("/pinnedMessageProperties/messageId")
                        .map(|_| str_of(&it["pinnedMessageProperties"], "messageId"))
                        .unwrap_or_else(|| str_of(it, "pinBarItemId")),
                    folder_id: folder.to_string(),
                    pinned_at: it
                        .pointer("/pinnedMessageProperties/pinnedDateTime")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                }),
                Some("folder") => {
                    let id = str_of(it, "pinBarItemId");
                    walk(&it["children"], &id, out);
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&v["children"], "", &mut out);
    out
}

/// Value of `consumptionHorizonBookmark` that marks a chat unread from `previous_id` on.
pub fn unread_bookmark(previous_id: &str, now: i64, client_message_id: &str) -> String {
    format!("{previous_id};{now};{client_message_id}")
}

/// Value of `consumptionHorizonBookmark` that clears a manual unread mark.
pub fn cleared_bookmark(now: i64) -> String {
    format!("0;{now};0")
}

impl Client {
    async fn conv_props_url(&self, conversation_id: &str, name: &str) -> String {
        format!(
            "{}/v1/users/ME/conversations/{}/properties?name={name}",
            self.chat_service().await,
            encode_conversation(conversation_id)
        )
    }

    async fn put_conv_prop(&self, conversation_id: &str, name: &str, body: Value) -> Result<()> {
        let url = self.conv_props_url(conversation_id, name).await;
        self.json("conversation property", Method::PUT, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Forwards up to five messages of `source` to each target, with an optional note.
    pub async fn forward(
        &self,
        source: &str,
        message_ids: &[String],
        targets: &[String],
        note_html: Option<&str>,
        display_name: &str,
    ) -> Result<Vec<Forwarded>> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages/forward",
            self.chat_service().await,
            encode_conversation(source)
        );
        let body = forward_body(
            message_ids,
            targets,
            note_html,
            display_name,
            &now_ms().to_string(),
        );
        let v = self
            .json("forward", Method::POST, &url, Some(&body))
            .await?;
        Ok(parse_forwarded(&v))
    }

    /// Saves or unsaves a message. `root_id` is the parent post for a channel reply, else the
    /// message id itself.
    pub async fn set_saved(
        &self,
        conversation_id: &str,
        root_id: &str,
        message_id: &str,
        saved: bool,
    ) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/rcmetadata/{root_id}",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let mid: Value = message_id
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::from(message_id));
        let body = json!({"s": if saved { 1 } else { 0 }, "mid": mid});
        self.json("save", Method::PUT, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Messages the user saved, newest first.
    pub async fn saved_messages(&self) -> Result<Vec<SavedMessage>> {
        let url = format!(
            "{}/v1/users/ME/conversations/48%3Asaved/messages?view=msnp24Equivalent&pageSize=200&startTime=1",
            self.chat_service().await
        );
        let v = self.json("saved", Method::GET, &url, None).await?;
        let mut out = parse_saved(&v);
        out.sort_by_key(|s| std::cmp::Reverse(s.saved_ms));
        Ok(out)
    }

    /// Messages pinned for everyone in a chat.
    pub async fn pinned(&self, chat_id: &str) -> Result<Vec<PinnedMessage>> {
        let url = format!("{CSA_V1}/chats/{}/pinbar", encode_conversation(chat_id));
        let v = self.json("pinbar", Method::GET, &url, None).await?;
        Ok(parse_pinbar(&v))
    }

    /// Pins (`true`) or unpins a chat message for everyone.
    pub async fn set_pinned(&self, chat_id: &str, message_id: &str, pinned: bool) -> Result<()> {
        let folder = if pinned {
            String::new()
        } else {
            self.pinned(chat_id)
                .await?
                .into_iter()
                .find(|p| p.message_id == message_id)
                .map(|p| p.folder_id)
                .unwrap_or_default()
        };
        let url = format!("{CSA_V1}/chats/{}/pinbar", encode_conversation(chat_id));
        let method = if pinned { Method::POST } else { Method::DELETE };
        let body = pinbar_body(chat_id, message_id, &folder, pinned);
        self.json("pin", method, &url, Some(&body))
            .await
            .map(|_| ())
    }

    /// Marks a conversation unread starting at the message after `previous_id`.
    pub async fn mark_unread(
        &self,
        conversation_id: &str,
        previous_id: &str,
        client_message_id: &str,
    ) -> Result<()> {
        let value = unread_bookmark(previous_id, now_ms(), client_message_id);
        self.put_conv_prop(
            conversation_id,
            "consumptionHorizonBookmark",
            json!({"consumptionHorizonBookmark": value}),
        )
        .await
    }

    /// Clears a manual "Mark as unread". A read horizon alone does not clear it.
    pub async fn clear_unread_mark(&self, conversation_id: &str) -> Result<()> {
        self.put_conv_prop(
            conversation_id,
            "consumptionHorizonBookmark",
            json!({"consumptionHorizonBookmark": cleared_bookmark(now_ms())}),
        )
        .await
    }

    /// Mutes (`true`) or unmutes notifications for a chat.
    pub async fn set_muted(&self, conversation_id: &str, muted: bool) -> Result<()> {
        self.put_conv_prop(
            conversation_id,
            "alerts",
            json!({"alerts": if muted { "false" } else { "true" }}),
        )
        .await
    }

    /// Hides a chat until its next message, or shows it again.
    pub async fn set_hidden(&self, conversation_id: &str, hidden: bool) -> Result<()> {
        if hidden {
            let now = now_ms();
            self.put_conv_prop(
                conversation_id,
                "unpinnedTime",
                json!({"unpinnedTime": now}),
            )
            .await?;
            self.put_conv_prop(
                conversation_id,
                "historyHiddenTime",
                json!({"historyHiddenTime": now.to_string()}),
            )
            .await
        } else {
            self.put_conv_prop(
                conversation_id,
                "unpinnedTime",
                json!({"unpinnedTime": null}),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_caps_at_five_ids() {
        let ids: Vec<String> = (1..=7).map(|i| i.to_string()).collect();
        let b = forward_body(&ids, &["48:notes".into()], Some("<p>fyi</p>"), "Me", "9");
        assert_eq!(b["messageIds"].as_array().unwrap().len(), 5);
        assert_eq!(b["targetThreadIds"][0], "48:notes");
        assert_eq!(b["additionalMessage"]["content"], "<p>fyi</p>");
        assert_eq!(b["templateId"], "basic_forward_message_template");
    }

    #[test]
    fn parses_forward_answer() {
        let v: Value = serde_json::from_str(
            r#"[{"statusCode":201,"threadId":"48:notes","messageId":"1795"},{"statusCode":403,"threadId":"19:x"}]"#,
        )
        .unwrap();
        let r = parse_forwarded(&v);
        assert_eq!(r[0].status, 201);
        assert_eq!(r[0].message_id.as_deref(), Some("1795"));
        assert_eq!(r[1].message_id, None);
    }

    #[test]
    fn parses_saved_activity_as_string_or_object() {
        let v = json!({"messages":[
            {"properties":{"activity":"{\"activityType\":\"savedMessage\",\"sourceMessageId\":\"5\",\"sourceThreadId\":\"19:a\",\"sourceReplyChainId\":\"0\",\"sourceUserImDisplayName\":\"A\",\"messagePreview\":\"hi\",\"activityTimestamp\":\"2026-11-18T10:00:00.000Z\"}"}},
            {"properties":{"activity":{"activityType":"savedMessage","sourceMessageId":"7","sourceThreadId":"19:c@thread.tacv2","sourceReplyChainId":"6","messagePreview":"re"}}},
            {"properties":{"activity":{"activityType":"mention"}}},
            {"properties":{}}
        ]});
        let s = parse_saved(&v);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].root_id, "5");
        assert_eq!(s[0].sender_name, "A");
        assert!(s[0].saved_ms > 0);
        assert_eq!(s[1].root_id, "6");
        assert_eq!(s[1].message_id, "7");
    }

    #[test]
    fn pinbar_flattens_folders() {
        let v = json!({"children":[
            {"type":"pinmsg","pinBarItemId":"1","pinnedMessageProperties":{"messageId":"1","pinnedDateTime":"t"}},
            {"type":"folder","pinBarItemId":"F","children":[{"type":"pinmsg","pinBarItemId":"2"}]}
        ]});
        let p = parse_pinbar(&v);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].folder_id, "");
        assert_eq!(p[1].message_id, "2");
        assert_eq!(p[1].folder_id, "F");
        let b = pinbar_body("19:a", "1", "", false);
        assert_eq!(b["pinbarOperation"], "remove");
    }

    #[test]
    fn bookmark_values() {
        assert_eq!(unread_bookmark("10", 99, "c"), "10;99;c");
        assert_eq!(cleared_bookmark(99), "0;99;0");
    }
}
