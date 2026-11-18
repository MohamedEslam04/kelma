//! @mentions: the HTML span Teams renders, the `properties.mentions` JSON that makes it notify,
//! and parsing of mentions on incoming messages.
//!
//! Both halves are required: name-only HTML does not notify, and an edit replaces
//! `properties`, so an edit must resend the mentions. Sources: xteams (verified live), Jezza/ic3.

use crate::chatsvc::{encode_conversation, send_body};
use crate::client::Client;
use crate::error::Result;
use crate::model::Message;
use crate::time::now_ms;
use reqwest::Method;
use serde_json::{Value, json};

const MENTION_TYPE: &str = "http://schema.skype.com/Mention";

/// What a mention points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentionKind {
    /// A person (`8:orgid:<oid>`). Verified live.
    Person,
    /// A whole team (`mentionType` `team`, mri the team thread). Seen in Teams web sends.
    Team,
    /// A channel (mri = the channel's own `19:…@thread.tacv2`). Verified live.
    Channel,
    /// "Everyone" in a group chat (mri = the chat id, `mentionType` `chat`). Unverified.
    Everyone,
    /// A team tag (mri `tag:<tagId>`).
    Tag,
}

impl MentionKind {
    fn wire(self) -> &'static str {
        match self {
            MentionKind::Person => "person",
            MentionKind::Team => "team",
            MentionKind::Channel => "channel",
            MentionKind::Everyone => "chat",
            MentionKind::Tag => "tag",
        }
    }

    fn from_wire(kind: Option<&str>, mri: &str) -> MentionKind {
        match kind {
            Some("person") => MentionKind::Person,
            Some("team") => MentionKind::Team,
            Some("channel") => MentionKind::Channel,
            Some("chat" | "everyone") => MentionKind::Everyone,
            Some("tag") => MentionKind::Tag,
            _ if mri.starts_with("tag:") => MentionKind::Tag,
            _ if mri.starts_with("19:") => MentionKind::Channel,
            _ => MentionKind::Person,
        }
    }
}

/// One mention in a message. `item_id` is the 0-based position of its span in the HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub item_id: u32,
    pub mri: String,
    pub display_name: String,
    pub kind: MentionKind,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The HTML Teams web sends for a mention chip. `item_id` counts spans in document order.
pub fn mention_html(item_id: u32, name: &str) -> String {
    format!(
        r#"<readonly class="skipProofing" itemtype="{MENTION_TYPE}" spellcheck="false"><span itemtype="{MENTION_TYPE}" itemscope="" itemid="{item_id}">{}</span></readonly>"#,
        escape(name)
    )
}

/// Value of `properties.mentions`: a JSON array encoded as a string.
pub fn mentions_property(mentions: &[Mention]) -> String {
    let items: Vec<Value> = mentions
        .iter()
        .map(|m| {
            json!({
                "@type": MENTION_TYPE,
                "itemid": m.item_id,
                "mri": m.mri,
                "mentionType": m.kind.wire(),
                "displayName": m.display_name,
            })
        })
        .collect();
    Value::Array(items).to_string()
}

/// Request body for sending a message with mentions (and an optional channel post subject).
pub fn send_body_with_mentions(
    html: &str,
    display_name: &str,
    client_message_id: &str,
    subject: Option<&str>,
    mentions: &[Mention],
) -> Value {
    let mut b = send_body(html, display_name, client_message_id, subject);
    if !mentions.is_empty() {
        b["properties"]["mentions"] = Value::String(mentions_property(mentions));
    }
    b
}

/// Request body for editing a message that has mentions. The edit replaces `properties`.
pub fn edit_body_with_mentions(html: &str, display_name: &str, mentions: &[Mention]) -> Value {
    let mut b = json!({
        "content": html,
        "messagetype": "RichText/Html",
        "contenttype": "text",
        "imdisplayname": display_name,
    });
    if !mentions.is_empty() {
        b["properties"] = json!({ "mentions": mentions_property(mentions) });
    }
    b
}

/// Parses `properties.mentions`, which arrives as an array or as a JSON string.
pub fn parse_mentions(props: Option<&Value>) -> Vec<Mention> {
    let raw = match props.and_then(|p| p.get("mentions")) {
        Some(Value::String(s)) if !s.is_empty() => serde_json::from_str::<Value>(s).ok(),
        Some(Value::String(_)) | Some(Value::Null) | None => None,
        Some(other) => Some(other.clone()),
    };
    raw.as_ref()
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let mri = m.get("mri").and_then(Value::as_str)?.to_string();
                    let item_id = match m.get("itemid")? {
                        Value::Number(n) => n.as_u64()? as u32,
                        Value::String(s) => s.parse().ok()?,
                        _ => return None,
                    };
                    let kind =
                        MentionKind::from_wire(m.get("mentionType").and_then(Value::as_str), &mri);
                    Some(Mention {
                        item_id,
                        display_name: m
                            .get("displayName")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        mri,
                        kind,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// True when `m` mentions `self_mri` by name or mentions everyone in the chat.
pub fn mentions_me(m: &Message, self_mri: &str) -> bool {
    m.mentions.iter().any(|x| match x.kind {
        MentionKind::Person => x.mri == self_mri,
        MentionKind::Everyone => true,
        _ => false,
    })
}

impl Client {
    /// Sends an HTML message whose `mentions` notify the people they name. The HTML must hold one
    /// [`mention_html`] span per entry. Returns the client message id used.
    pub async fn send_message_with_mentions(
        &self,
        conversation_id: &str,
        html: &str,
        display_name: &str,
        subject: Option<&str>,
        mentions: &[Mention],
    ) -> Result<String> {
        let cmid = now_ms().to_string();
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let body = send_body_with_mentions(html, display_name, &cmid, subject, mentions);
        self.json("send", Method::POST, &url, Some(&body)).await?;
        Ok(cmid)
    }

    /// Edits one of our messages and resends its mentions (an edit would otherwise drop them).
    pub async fn edit_message_with_mentions(
        &self,
        conversation_id: &str,
        message_id: &str,
        html: &str,
        display_name: &str,
        mentions: &[Mention],
    ) -> Result<()> {
        let url = format!(
            "{}/v1/users/ME/conversations/{}/messages/{message_id}",
            self.chat_service().await,
            encode_conversation(conversation_id)
        );
        let body = edit_body_with_mentions(html, display_name, mentions);
        self.json("edit", Method::PUT, &url, Some(&body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mention(id: u32, mri: &str, name: &str, kind: MentionKind) -> Mention {
        Mention {
            item_id: id,
            mri: mri.into(),
            display_name: name.into(),
            kind,
        }
    }

    #[test]
    fn html_wraps_the_name_in_a_mention_span() {
        let h = mention_html(0, "Sara <Test> & Co");
        assert!(h.contains(r#"itemtype="http://schema.skype.com/Mention""#));
        assert!(h.contains(r#"itemid="0""#));
        assert!(h.contains(">Sara &lt;Test&gt; &amp; Co</span>"));
        assert!(h.starts_with("<readonly"));
        assert!(h.ends_with("</readonly>"));
    }

    #[test]
    fn property_is_a_json_string_with_kinds() {
        let ms = [
            mention(0, "8:orgid:aaa", "Sara", MentionKind::Person),
            mention(1, "19:chan@thread.tacv2", "General", MentionKind::Channel),
            mention(2, "19:team@thread.tacv2", "Team A", MentionKind::Team),
            mention(3, "tag:t1", "Leads", MentionKind::Tag),
            mention(4, "19:grp@thread.v2", "Everyone", MentionKind::Everyone),
        ];
        let s = mentions_property(&ms);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v[0]["@type"], "http://schema.skype.com/Mention");
        assert_eq!(v[0]["itemid"], 0);
        assert_eq!(v[0]["mri"], "8:orgid:aaa");
        assert_eq!(v[0]["mentionType"], "person");
        assert_eq!(v[0]["displayName"], "Sara");
        assert_eq!(v[1]["mentionType"], "channel");
        assert_eq!(v[2]["mentionType"], "team");
        assert_eq!(v[3]["mentionType"], "tag");
        assert_eq!(v[4]["mentionType"], "chat");
    }

    #[test]
    fn body_carries_mentions_and_subject() {
        let ms = [mention(0, "8:orgid:aaa", "Sara", MentionKind::Person)];
        let b = send_body_with_mentions("<p>hi</p>", "Me", "123", Some("Subj"), &ms);
        assert_eq!(b["content"], "<p>hi</p>");
        assert_eq!(b["messagetype"], "RichText/Html");
        assert_eq!(b["properties"]["subject"], "Subj");
        assert!(b["properties"]["mentions"].is_string());
        let none = send_body_with_mentions("x", "Me", "1", None, &[]);
        assert!(none.get("properties").is_none());
    }

    #[test]
    fn edit_body_resends_mentions() {
        let ms = [mention(0, "8:orgid:aaa", "Sara", MentionKind::Person)];
        let b = edit_body_with_mentions("<p>x</p>", "Me", &ms);
        assert_eq!(b["content"], "<p>x</p>");
        assert!(b["properties"]["mentions"].is_string());
    }

    #[test]
    fn parses_array_and_string_forms() {
        let arr = json!({"mentions": [
            {"@type": "http://schema.skype.com/Mention", "itemid": 0, "mri": "8:orgid:aaa", "mentionType": "person", "displayName": "Sara"},
            {"itemid": "1", "mri": "tag:t1", "mentionType": "tag", "displayName": "Leads"},
            {"itemid": 2, "mri": "19:c@thread.tacv2", "mentionType": "channel", "displayName": "General"},
            {"itemid": 3, "mri": "19:g@thread.v2", "mentionType": "chat", "displayName": "Everyone"},
            {"itemid": 4, "displayName": "no mri"}
        ]});
        let p = parse_mentions(Some(&arr));
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].kind, MentionKind::Person);
        assert_eq!(p[1].item_id, 1);
        assert_eq!(p[1].kind, MentionKind::Tag);
        assert_eq!(p[2].kind, MentionKind::Channel);
        assert_eq!(p[3].kind, MentionKind::Everyone);
        let as_string = json!({"mentions": arr["mentions"].to_string()});
        assert_eq!(parse_mentions(Some(&as_string)), p);
        assert!(parse_mentions(None).is_empty());
        assert!(parse_mentions(Some(&json!({"mentions": ""}))).is_empty());
        assert!(parse_mentions(Some(&json!({"mentions": "not json"}))).is_empty());
    }

    #[test]
    fn unknown_type_falls_back_to_the_mri_prefix() {
        let v = json!({"mentions": [
            {"itemid": 0, "mri": "8:orgid:x", "mentionType": "weird"},
            {"itemid": 1, "mri": "tag:y"},
        ]});
        let p = parse_mentions(Some(&v));
        assert_eq!(p[0].kind, MentionKind::Person);
        assert_eq!(p[1].kind, MentionKind::Tag);
    }

    #[test]
    fn mentions_me_matches_person_and_everyone() {
        let me = "8:orgid:me";
        let mut m = crate::model::message_from_json(
            &json!({"id": "1", "from": "8:orgid:other", "messagetype": "RichText/Html", "content": "x"}),
            "19:c@thread.v2",
        )
        .unwrap();
        assert!(!mentions_me(&m, me));
        m.mentions = vec![mention(0, "8:orgid:other2", "O", MentionKind::Person)];
        assert!(!mentions_me(&m, me));
        m.mentions
            .push(mention(1, "8:orgid:me", "Me", MentionKind::Person));
        assert!(mentions_me(&m, me));
        m.mentions = vec![mention(
            0,
            "19:g@thread.v2",
            "Everyone",
            MentionKind::Everyone,
        )];
        assert!(mentions_me(&m, me));
        m.mentions = vec![mention(0, "tag:me", "Tag", MentionKind::Tag)];
        assert!(!mentions_me(&m, me));
    }

    #[test]
    fn message_parser_fills_mentions() {
        let m = crate::model::message_from_json(
            &json!({"id": "1", "from": "8:orgid:other", "messagetype": "RichText/Html", "content": "x",
                "properties": {"mentions": "[{\"itemid\":0,\"mri\":\"8:orgid:me\",\"mentionType\":\"person\",\"displayName\":\"Me\"}]"}}),
            "19:c@thread.v2",
        )
        .unwrap();
        assert!(mentions_me(&m, "8:orgid:me"));
    }
}
