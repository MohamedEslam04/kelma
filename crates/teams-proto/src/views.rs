//! Quick views and shared files: followed channel threads (`48:threads`), following and
//! unfollowing a thread, and the files and links shared in a conversation (Substrate `AllShared`).
//! Sources: Maxim `findings.md` and m0nkmaster `API-REFERENCE.md` (`docs/research/open-source.md`).

use crate::broker::Audience;
use crate::chatsvc::encode_conversation;
use crate::client::Client;
use crate::client_info::TEAMS_ORIGIN;
use crate::error::{Error, Result};
use crate::model::split_thread;
use crate::people::{anchor_mailbox, oid_of_mri};
use crate::time::parse_iso_ms;
use reqwest::Method;
use serde_json::{Value, json};

const FOLLOWED: &str = "48:threads";

/// A channel thread the user follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowedThread {
    /// The channel (a team's General channel is the team id).
    pub conversation_id: String,
    /// The root post of the thread.
    pub root_id: String,
    /// `Team > channel`, as the feed words it.
    pub topic: String,
    /// When the feed row was written.
    pub since_ms: i64,
    /// Teams followed it by itself (you started it or replied) rather than you asking to.
    pub implicit: bool,
}

fn text(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// `properties.activity` of a feed row (an object, or a JSON string holding one).
fn activity_of(row: &Value) -> Option<Value> {
    match row.pointer("/properties/activity")? {
        Value::String(s) => serde_json::from_str(s).ok().filter(Value::is_object),
        v if v.is_object() => Some(v.clone()),
        _ => None,
    }
}

/// Parses `48:threads` rows. A thread has several rows over time; the newest decides, and a
/// `deleted` row means it is no longer followed. Newest thread first.
pub fn parse_followed(v: &Value) -> Vec<FollowedThread> {
    let rows = v.get("messages").unwrap_or(v);
    let mut keyed: Vec<(i64, FollowedThread, bool)> = vec![];
    for row in rows.as_array().into_iter().flatten() {
        let Some(a) = activity_of(row) else { continue };
        let (conv, _) = split_thread(&text(&a, "sourceThreadId"));
        let root = text(&a, "sourceReplyChainId");
        if conv.is_empty() || root.is_empty() {
            continue;
        }
        let since = text(row, "id").parse().unwrap_or(0);
        let deleted = text(&a, "activityReason").eq_ignore_ascii_case("deleted");
        let implicit = a
            .pointer("/activityContext/followType")
            .and_then(Value::as_str)
            .is_some_and(|t| t.eq_ignore_ascii_case("ImplicitFollow"));
        keyed.push((
            since,
            FollowedThread {
                conversation_id: conv,
                root_id: root,
                topic: text(&a, "sourceThreadTopic"),
                since_ms: since,
                implicit,
            },
            deleted,
        ));
    }
    keyed.sort_by_key(|(since, ..)| std::cmp::Reverse(*since));
    let mut out: Vec<FollowedThread> = vec![];
    let mut seen: Vec<(String, String)> = vec![];
    for (_, t, deleted) in keyed {
        let key = (t.conversation_id.clone(), t.root_id.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        if !deleted {
            out.push(t);
        }
    }
    out
}

/// Body of the `emotions` property write that follows (`"0"`) or unfollows (`"1"`) a post.
pub fn follow_body(follow: bool) -> Value {
    json!({"emotions": {"key": "follow", "value": if follow { "0" } else { "1" }}})
}

/// What a shared item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedKind {
    File,
    Link,
}

/// A file or link shared in a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedItem {
    pub kind: SharedKind,
    /// The file name, or the link's title (its address when it has none).
    pub name: String,
    /// Where it opens.
    pub url: String,
    /// File extension without the dot, empty for links.
    pub extension: String,
    pub shared_by: String,
    pub shared_by_mri: String,
    pub shared_ms: i64,
    pub message_id: String,
}

/// Parses the answer of `AllShared`, newest first.
pub fn parse_shared(v: &Value) -> Vec<SharedItem> {
    let mut items: Vec<SharedItem> = v
        .get("Items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let (kind, name, url, extension) =
                if let Some(f) = i.get("FileData").filter(|f| f.is_object()) {
                    let url = [text(f, "WebUrl"), text(f, "FileUrl")]
                        .into_iter()
                        .find(|u| !u.is_empty())?;
                    (
                        SharedKind::File,
                        text(f, "FileName"),
                        url,
                        text(f, "FileExtension").trim_start_matches('.').to_string(),
                    )
                } else {
                    let l = i.get("WeblinkData").filter(|l| l.is_object())?;
                    let url = text(l, "WebUrl");
                    if url.is_empty() {
                        return None;
                    }
                    let title = text(l, "Title");
                    let name = if title.is_empty() { url.clone() } else { title };
                    (SharedKind::Link, name, url, String::new())
                };
            Some(SharedItem {
                kind,
                name,
                url,
                extension,
                shared_by: text(i, "SharedByDisplayName"),
                shared_by_mri: text(i, "SharedByMRIId"),
                shared_ms: parse_iso_ms(&text(i, "SharedDateTime")).unwrap_or(0),
                message_id: text(i, "TeamsMessageId"),
            })
        })
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.shared_ms));
    items
}

/// The `AllShared` URL for a conversation.
pub fn shared_url(oid: &str, tenant: &str, conversation: &str, page_size: u32) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    format!(
        "https://substrate.office.com/AllFiles/api/users('OID:{oid}@{tenant}')/AllShared?ThreadId={}&ItemTypes=File&ItemTypes=Link&PageSize={page_size}",
        enc(conversation)
    )
}

impl Client {
    async fn conversation_url(&self, id: &str) -> String {
        format!(
            "{}/v1/users/ME/conversations/{}",
            self.chat_service().await,
            encode_conversation(id)
        )
    }

    /// The channel threads the user follows, newest first.
    pub async fn followed_threads(&self, page_size: u32) -> Result<Vec<FollowedThread>> {
        let url = format!(
            "{}/messages?view=msnp24Equivalent|supportsMessageProperties&pageSize={page_size}&startTime=1",
            self.conversation_url(FOLLOWED).await
        );
        let v = self
            .json("followed threads", Method::GET, &url, None)
            .await?;
        Ok(parse_followed(&v))
    }

    /// Follows or unfollows the thread of a channel post. Verified live: the service stores the
    /// `follow` emotion on the post (`"0"` follow, `"1"` unfollow, a `DELETE` clears it).
    pub async fn set_thread_follow(
        &self,
        channel_id: &str,
        root_id: &str,
        follow: bool,
    ) -> Result<()> {
        let url = format!(
            "{}/messages/{root_id}/properties?name=emotions",
            self.conversation_url(channel_id).await
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

    /// The files and links shared in a chat or channel, newest first (first page of 50).
    pub async fn shared_items(
        &self,
        self_mri: &str,
        conversation: &str,
    ) -> Result<Vec<SharedItem>> {
        let tenant = self.tenant_id().await?;
        let anchor = anchor_mailbox(self_mri, &tenant)
            .ok_or_else(|| Error::Protocol(format!("{self_mri} is not an orgid MRI")))?;
        let oid = oid_of_mri(self_mri).unwrap_or_default();
        let url = shared_url(oid, &tenant, conversation, 50);
        let (status, answer) = self
            .request_as(
                &Audience::Substrate,
                Method::GET,
                &url,
                None,
                &[
                    ("Origin", TEAMS_ORIGIN),
                    ("Referer", &format!("{TEAMS_ORIGIN}/")),
                    ("x-anchormailbox", &anchor),
                ],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("shared items", status, &answer));
        }
        Ok(parse_shared(&serde_json::from_str(&answer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, thread: &str, chain: &str, reason: &str, follow: Option<&str>) -> Value {
        json!({
            "id": id,
            "properties": {"activity": {
                "activityType": "threads",
                "activityReason": reason,
                "sourceThreadId": thread,
                "sourceReplyChainId": chain,
                "sourceThreadTopic": "Team > Channel",
                "activityContext": {"followType": follow}
            }}
        })
    }

    #[test]
    fn newest_row_decides_and_deleted_rows_drop_the_thread() {
        let v = json!({"messages": [
            row("100", "19:c@thread.tacv2", "90", "author", Some("ImplicitFollow")),
            row("300", "19:c@thread.tacv2", "50", "deleted", None),
            row("200", "19:c@thread.tacv2", "50", "channelFollow", Some("ExplicitFollow")),
            row("250", "19:d@thread.tacv2;messageid=7", "70", "author", Some("ImplicitFollow")),
        ]});
        let t = parse_followed(&v);
        assert_eq!(t.len(), 2, "the thread with a newer deleted row is gone");
        assert_eq!(t[0].conversation_id, "19:d@thread.tacv2");
        assert_eq!(t[0].root_id, "70");
        assert!(t[0].implicit);
        assert_eq!((t[1].root_id.as_str(), t[1].since_ms), ("90", 100));
    }

    #[test]
    fn rows_without_a_thread_or_root_are_skipped() {
        let v = json!({"messages": [
            row("1", "", "9", "author", None),
            row("2", "19:c@thread.tacv2", "", "author", None),
            json!({"id": "3", "properties": {}}),
        ]});
        assert!(parse_followed(&v).is_empty());
    }

    #[test]
    fn follow_writes_the_follow_emotion() {
        assert_eq!(
            follow_body(true),
            json!({"emotions": {"key": "follow", "value": "0"}})
        );
        assert_eq!(follow_body(false)["emotions"]["value"], "1");
    }

    #[test]
    fn shared_items_cover_files_and_links_newest_first() {
        let v = json!({"Items": [
            {
                "FileData": {"FileName": "plan.pdf", "FileUrl": "https://x/plan.pdf", "FileExtension": "pdf"},
                "WeblinkData": null,
                "SharedByDisplayName": "Sara", "SharedByMRIId": "8:orgid:u1",
                "SharedDateTime": "2026-11-17T10:00:00.0000000+00:00", "TeamsMessageId": "11"
            },
            {
                "FileData": null,
                "WeblinkData": {"Title": "", "WebUrl": "https://example.com/a"},
                "SharedByDisplayName": "Omar",
                "SharedDateTime": "2026-11-18T10:00:00.0000000+00:00", "TeamsMessageId": "12"
            },
            {"FileData": null, "WeblinkData": null}
        ]});
        let items = parse_shared(&v);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, SharedKind::Link);
        assert_eq!(
            items[0].name, "https://example.com/a",
            "a link without a title shows its address"
        );
        assert_eq!(items[1].kind, SharedKind::File);
        assert_eq!(
            (items[1].name.as_str(), items[1].extension.as_str()),
            ("plan.pdf", "pdf")
        );
        assert_eq!(items[1].shared_by, "Sara");
        assert!(items[0].shared_ms > items[1].shared_ms);
    }

    #[test]
    fn shared_url_encodes_the_conversation() {
        let u = shared_url("oid", "tid", "19:abc@thread.v2", 25);
        assert!(u.contains("users('OID:oid@tid')/AllShared?ThreadId=19%3Aabc%40thread.v2"));
        assert!(u.ends_with("&PageSize=25"));
    }
}
