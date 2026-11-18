//! Creating group chats and managing their roster and topic.
//!
//! Endpoints are in `docs/research/open-source.md` §3.3 (better-teams, purple-teams, Jezza).
//! None of these have been run live against a shared chat: they are visible to other people.

use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::error::{Error, Result};
use reqwest::Method;
use serde_json::{Value, json};

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// The deterministic id of the 1:1 chat between two people: `19:<low>_<high>@unq.gbl.spaces`
/// with both AAD object ids lowercased and sorted. It needs no create call.
pub fn one_on_one_id(oid_a: &str, oid_b: &str) -> String {
    let (a, b) = (oid_a.to_ascii_lowercase(), oid_b.to_ascii_lowercase());
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    format!("19:{lo}_{hi}@unq.gbl.spaces")
}

/// Body for `POST /v1/threads` creating a group chat. The creator comes first and every member
/// is an admin, which is how Teams lets any member manage a group chat.
pub fn create_group_body(self_mri: &str, members: &[String], topic: Option<&str>) -> Value {
    let mut roster = vec![json!({"id": self_mri, "role": "Admin"})];
    roster.extend(
        members
            .iter()
            .filter(|m| m.as_str() != self_mri)
            .map(|m| json!({"id": m, "role": "Admin"})),
    );
    let mut props = json!({"threadType": "chat", "chatFilesIndexId": "2"});
    if let Some(t) = topic.filter(|t| !t.is_empty()) {
        props["topic"] = Value::from(t);
    }
    json!({"members": roster, "properties": props})
}

/// Body for renaming a thread. An empty topic clears the name.
pub fn rename_body(topic: &str) -> Value {
    json!({ "topic": topic })
}

/// Body for adding a member. `share_history_ms` is the `shareHistoryTime`: 0 shares the whole
/// history, the current time shares none, and now minus N days shares the last N days.
pub fn add_member_body(share_history_ms: i64) -> Value {
    json!({"role": "Admin", "shareHistoryTime": share_history_ms})
}

/// The thread id at the end of a `Location` header such as `https://…/v1/threads/19%3Aabc%40thread.v2`.
pub fn thread_id_from_location(location: &str) -> Option<String> {
    let last = location.split('?').next()?.rsplit('/').next()?;
    let decoded = percent_decode(last);
    decoded.starts_with("19:").then_some(decoded)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Client {
    async fn thread_url(&self, thread_id: &str) -> String {
        format!(
            "{}/v1/threads/{}",
            self.chat_service().await,
            encode_conversation(thread_id)
        )
    }

    /// Creates a group chat with `members` (MRIs, without ourselves) and returns its thread id.
    /// A group needs at least two other people: for one person use [`one_on_one_id`].
    pub async fn create_group_chat(
        &self,
        members: &[String],
        topic: Option<&str>,
    ) -> Result<String> {
        let me = self.self_mri().await?;
        let body = create_group_body(&me, members, topic);
        let url = format!("{}/v1/threads", self.chat_service().await);
        let (status, text, location) = self
            .request_location(Method::POST, &url, Some(&body), &[])
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("create group chat", status, &text));
        }
        location
            .as_deref()
            .and_then(thread_id_from_location)
            .ok_or_else(|| Error::Protocol("thread created but no id in Location".into()))
    }

    /// Sets the topic (group name) of a thread.
    pub async fn rename_thread(&self, thread_id: &str, topic: &str) -> Result<()> {
        let url = format!("{}/properties?name=topic", self.thread_url(thread_id).await);
        self.json("rename", Method::PUT, &url, Some(&rename_body(topic)))
            .await
            .map(|_| ())
    }

    /// Adds a person to a group chat. See [`add_member_body`] for `share_history_ms`.
    pub async fn add_member(
        &self,
        thread_id: &str,
        mri: &str,
        share_history_ms: i64,
    ) -> Result<()> {
        let url = format!("{}/members/{}", self.thread_url(thread_id).await, enc(mri));
        self.json(
            "add member",
            Method::PUT,
            &url,
            Some(&add_member_body(share_history_ms)),
        )
        .await
        .map(|_| ())
    }

    /// Removes our own membership from a group chat.
    pub async fn leave_thread(&self, thread_id: &str, self_mri: &str) -> Result<()> {
        let url = format!(
            "{}/members/{}",
            self.thread_url(thread_id).await,
            enc(self_mri)
        );
        self.json("leave", Method::DELETE, &url, None)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_on_one_id_is_lowercase_and_sorted() {
        let a = "AAAA1111-0000-0000-0000-000000000001";
        let b = "bbbb2222-0000-0000-0000-000000000002";
        let want = "19:aaaa1111-0000-0000-0000-000000000001_bbbb2222-0000-0000-0000-000000000002@unq.gbl.spaces";
        assert_eq!(one_on_one_id(a, b), want);
        assert_eq!(one_on_one_id(b, a), want);
        // A chat with yourself repeats the id.
        assert_eq!(one_on_one_id("X", "x"), "19:x_x@unq.gbl.spaces");
    }

    #[test]
    fn group_body_puts_the_creator_first_and_skips_duplicates() {
        let members = vec![
            "8:orgid:b".to_string(),
            "8:orgid:me".to_string(),
            "8:orgid:c".to_string(),
        ];
        let b = create_group_body("8:orgid:me", &members, Some("Project X"));
        let r = b["members"].as_array().unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0]["id"], "8:orgid:me");
        assert!(r.iter().all(|m| m["role"] == "Admin"));
        assert_eq!(b["properties"]["threadType"], "chat");
        assert_eq!(b["properties"]["topic"], "Project X");
        let no_topic = create_group_body("8:orgid:me", &members, Some(""));
        assert!(no_topic["properties"].get("topic").is_none());
    }

    #[test]
    fn member_and_rename_bodies() {
        assert_eq!(rename_body("New"), json!({"topic": "New"}));
        let b = add_member_body(0);
        assert_eq!(b["role"], "Admin");
        assert_eq!(b["shareHistoryTime"], 0);
        assert_eq!(add_member_body(123)["shareHistoryTime"], 123);
    }

    #[test]
    fn thread_id_comes_from_the_location_header() {
        assert_eq!(
            thread_id_from_location(
                "https://x.ng.msg.teams.microsoft.com/v1/threads/19:abc@thread.v2"
            )
            .as_deref(),
            Some("19:abc@thread.v2")
        );
        assert_eq!(
            thread_id_from_location("https://x/v1/threads/19%3Aabc%40thread.v2?x=1").as_deref(),
            Some("19:abc@thread.v2")
        );
        assert_eq!(thread_id_from_location("https://x/v1/threads/"), None);
        assert_eq!(thread_id_from_location("https://x/v1/other/48:notes"), None);
    }

    #[test]
    fn member_mris_are_path_encoded() {
        assert_eq!(enc("8:orgid:aaa"), "8%3Aorgid%3Aaaa");
    }
}
