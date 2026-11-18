//! Viva Engage (Yammer): Communities and storyline posts, over `api.yammer.com` with the
//! `Audience::Yammer` token. Communities are read-only here.

use crate::broker::Audience;
use crate::client::Client;
use crate::error::{Error, Result};
use crate::time::parse_iso_ms;
use reqwest::Method;
use serde_json::{Value, json};

/// Base of the Yammer REST API.
pub const YAMMER_V1: &str = "https://api.yammer.com/api/v1";

/// A community the user belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Community {
    pub id: u64,
    pub name: String,
    pub description: String,
    /// `public` or `private`.
    pub privacy: String,
    pub members: u32,
    /// The company-wide community ("All Company").
    pub is_company_wide: bool,
    pub last_message_ms: i64,
    pub web_url: String,
}

/// One post (or reply) of a community feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngagePost {
    pub id: u64,
    pub thread_id: u64,
    /// The post this one replies to.
    pub replied_to_id: Option<u64>,
    pub sender_id: u64,
    pub sender_name: String,
    pub created_ms: i64,
    pub text: String,
    pub likes: u32,
    pub attachments: u32,
    pub web_url: String,
}

/// Yammer writes times as `2026/09/20 08:53:00 +0000`.
pub fn parse_yammer_time(s: &str) -> Option<i64> {
    let (date, rest) = s.trim().split_once(' ')?;
    let (clock, offset) = rest.split_once(' ').unwrap_or((rest, "+0000"));
    if offset.len() != 5 {
        return None;
    }
    let iso = format!(
        "{}T{clock}{}:{}",
        date.replace('/', "-"),
        &offset[..3],
        &offset[3..]
    );
    parse_iso_ms(&iso)
}

/// Parses `GET groups.json?mine=1`.
pub fn parse_communities(v: &Value) -> Vec<Community> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            Some(Community {
                id: g.get("id")?.as_u64()?,
                name: g
                    .get("full_name")
                    .or_else(|| g.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                description: g
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                privacy: g
                    .get("privacy")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                members: g
                    .pointer("/stats/members")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                is_company_wide: g
                    .get("company_group")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                last_message_ms: g
                    .pointer("/stats/last_message_at")
                    .and_then(Value::as_str)
                    .and_then(parse_yammer_time)
                    .unwrap_or_default(),
                web_url: g
                    .get("web_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
}

/// Parses `GET messages/in_group/<id>.json`: `messages` plus the users in `references`.
pub fn parse_feed(v: &Value) -> Vec<EngagePost> {
    let name_of = |id: u64| {
        v.get("references")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|r| {
                r.get("type").and_then(Value::as_str) == Some("user")
                    && r.get("id").and_then(Value::as_u64) == Some(id)
            })
            .and_then(|r| r.get("full_name").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };
    v.get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let sender_id = m.get("sender_id").and_then(Value::as_u64).unwrap_or(0);
            Some(EngagePost {
                id: m.get("id")?.as_u64()?,
                thread_id: m.get("thread_id").and_then(Value::as_u64).unwrap_or(0),
                replied_to_id: m.get("replied_to_id").and_then(Value::as_u64),
                sender_name: name_of(sender_id),
                sender_id,
                created_ms: m
                    .get("created_at")
                    .and_then(Value::as_str)
                    .and_then(parse_yammer_time)
                    .unwrap_or_default(),
                text: m
                    .pointer("/body/plain")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                likes: m
                    .pointer("/liked_by/count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
                attachments: m
                    .get("attachments")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len) as u32,
                web_url: m
                    .get("web_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
}

/// Body of a storyline post: a post on our own profile feed, with no community.
/// Not verified live (a storyline post is public to the company).
pub fn storyline_body(text: &str) -> Value {
    json!({"body": text, "is_storyline_post": true})
}

impl Client {
    /// The communities we belong to.
    pub async fn communities(&self) -> Result<Vec<Community>> {
        let v = self
            .json_as(
                "communities",
                &Audience::Yammer,
                Method::GET,
                &format!("{YAMMER_V1}/groups.json?mine=1"),
                None,
            )
            .await?;
        Ok(parse_communities(&v))
    }

    /// The newest posts of a community, newest first. `older_than` is the id of the oldest post
    /// of the previous page.
    pub async fn community_feed(
        &self,
        community_id: u64,
        limit: u32,
        older_than: Option<u64>,
    ) -> Result<Vec<EngagePost>> {
        let mut url = format!("{YAMMER_V1}/messages/in_group/{community_id}.json?limit={limit}");
        if let Some(o) = older_than {
            url.push_str(&format!("&older_than={o}"));
        }
        let v = self
            .json_as("community feed", &Audience::Yammer, Method::GET, &url, None)
            .await?;
        Ok(parse_feed(&v))
    }

    /// Posts to our storyline (best effort, the body is not verified against the service).
    pub async fn post_storyline(&self, text: &str) -> Result<Vec<EngagePost>> {
        if text.trim().is_empty() {
            return Err(Error::Protocol("a storyline post needs text".into()));
        }
        let v = self
            .json_as(
                "storyline post",
                &Audience::Yammer,
                Method::POST,
                &format!("{YAMMER_V1}/messages.json"),
                Some(&storyline_body(text)),
            )
            .await?;
        Ok(parse_feed(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yammer_times() {
        assert_eq!(
            parse_yammer_time("2026/09/20 08:53:00 +0000"),
            parse_iso_ms("2026-09-20T08:53:00Z")
        );
        assert_eq!(
            parse_yammer_time("2026/09/20 10:53:00 +0200"),
            parse_iso_ms("2026-09-20T08:53:00Z")
        );
        assert_eq!(parse_yammer_time("nonsense"), None);
        assert_eq!(parse_yammer_time(""), None);
    }

    #[test]
    fn parses_communities() {
        let c = parse_communities(&json!([
            {"id": 11, "name": "all", "full_name": "All Company", "description": "d",
             "privacy": "public", "company_group": true, "web_url": "https://x/y",
             "stats": {"members": 50, "last_message_at": "2026/09/20 08:53:00 +0000"}},
            {"name": "no id"}
        ]));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "All Company");
        assert_eq!((c[0].members, c[0].is_company_wide), (50, true));
        assert!(c[0].last_message_ms > 0);
        assert!(parse_communities(&json!({})).is_empty());
    }

    #[test]
    fn parses_a_feed_with_sender_names() {
        let posts = parse_feed(&json!({
            "messages": [
                {"id": 5, "thread_id": 5, "sender_id": 9, "replied_to_id": null,
                 "created_at": "2026/09/20 08:53:00 +0000",
                 "body": {"plain": "hello", "rich": "<b>hello</b>"},
                 "liked_by": {"count": 2}, "attachments": [{}], "web_url": "https://x/5"},
                {"id": 6, "thread_id": 5, "sender_id": 10, "replied_to_id": 5, "body": {}},
                {"sender_id": 9}
            ],
            "references": [
                {"type": "user", "id": 9, "full_name": "Ann Example"},
                {"type": "thread", "id": 9, "full_name": "not a user"}
            ]
        }));
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].sender_name, "Ann Example");
        assert_eq!(posts[0].text, "hello");
        assert_eq!((posts[0].likes, posts[0].attachments), (2, 1));
        assert_eq!(posts[1].replied_to_id, Some(5));
        assert_eq!(posts[1].sender_name, "");
    }

    #[test]
    fn storyline_body_marks_the_post() {
        let b = storyline_body("hi");
        assert_eq!(b["body"], "hi");
        assert_eq!(b["is_storyline_post"], true);
    }
}
