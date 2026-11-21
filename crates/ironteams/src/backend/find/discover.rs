//! The Discover quick view. Teams' Discover feed (popular posts from your teams) has no endpoint
//! documented in docs/protocol.md, docs/research or the reference clients, so IronTeams builds its own
//! from what it already stores: the channel posts of the last 7 days, ranked by their reactions
//! and replies. The view says so.

use super::Hit;
use crate::backend::Engine;
use serde_json::Value;
use std::sync::Arc;
use store::{ConvFilter, MessageRow};
use teams_proto::time::now_ms;

/// How far back Discover looks.
const DAYS: i64 = 7;
/// Channels scanned, posts read per channel and rows shown.
const CHANNELS: usize = 200;
const PER_CHANNEL: usize = 50;
const SHOWN: usize = 50;

/// Reactions on a message (its `emotions`), not counting the "follow" mark Teams keeps there.
pub(super) fn reaction_count(reactions_json: &str) -> usize {
    let props: Value = serde_json::from_str(reactions_json).unwrap_or_default();
    teams_proto::model::parse_reactions(Some(&props))
        .iter()
        .filter(|r| r.key != "follow")
        .map(|r| r.users.len())
        .sum()
}

/// "3 reactions · 1 reply", or "" when there are none.
pub(super) fn engagement(reactions: usize, replies: usize) -> String {
    let mut parts = Vec::new();
    match reactions {
        0 => {}
        1 => parts.push("1 reaction".to_string()),
        n => parts.push(format!("{n} reactions")),
    }
    match replies {
        0 => {}
        1 => parts.push("1 reply".to_string()),
        n => parts.push(format!("{n} replies")),
    }
    parts.join(" · ")
}

/// A scored post: reactions plus replies first, then the newest.
struct Scored {
    reactions: usize,
    replies: usize,
    row: MessageRow,
}

/// Sorts posts by reactions plus replies, the newest first among equals, and keeps the top.
fn rank(mut posts: Vec<Scored>) -> Vec<Scored> {
    posts.sort_by(|a, b| {
        (b.reactions + b.replies)
            .cmp(&(a.reactions + a.replies))
            .then(b.row.compose_ms.cmp(&a.row.compose_ms))
            .then(a.row.id.cmp(&b.row.id))
    });
    posts.truncate(SHOWN);
    posts
}

impl Engine {
    pub(super) fn load_discover(self: &Arc<Self>, seq: u64) {
        let since = now_ms() - DAYS * 24 * 3_600_000;
        let posts = {
            let st = self.store.lock().unwrap();
            let channels = st
                .conversations(ConvFilter::Channels, CHANNELS)
                .unwrap_or_default();
            let mut posts = Vec::new();
            for c in channels.iter().filter(|c| c.last_activity_ms >= since) {
                for m in st
                    .messages_before(&c.id, None, PER_CHANNEL)
                    .unwrap_or_default()
                {
                    if m.deleted
                        || m.compose_ms < since
                        || m.message_type.starts_with("Control/")
                        || m.message_type.starts_with("ThreadActivity/")
                    {
                        continue;
                    }
                    posts.push(Scored {
                        reactions: reaction_count(&m.reactions_json),
                        replies: st.reply_count(&m.id).unwrap_or(0),
                        row: m,
                    });
                }
            }
            posts
        };
        let hits: Vec<Hit> = rank(posts)
            .into_iter()
            .map(|p| {
                let mut h = self.hit_from_row(&p.row);
                let e = engagement(p.reactions, p.replies);
                if !e.is_empty() {
                    h.place = format!("{} · {e}", h.place);
                }
                h
            })
            .collect();
        {
            let mut f = self.find.lock().unwrap();
            if f.quick_seq != seq {
                return;
            }
            f.quick_hits = hits;
        }
        self.push_quick(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: &str, ms: i64, reactions: usize, replies: usize) -> Scored {
        Scored {
            reactions,
            replies,
            row: MessageRow {
                id: id.into(),
                conversation_id: "19:c@thread.tacv2".into(),
                parent_id: None,
                sender_mri: String::new(),
                sender_name: String::new(),
                compose_ms: ms,
                edited_ms: None,
                deleted: false,
                message_type: "RichText/Html".into(),
                content_html: String::new(),
                client_message_id: None,
                reactions_json: "{}".into(),
            },
        }
    }

    #[test]
    fn reactions_are_counted_without_the_follow_mark() {
        let json = r#"{"emotions":[{"key":"like","users":[{"mri":"8:a"},{"mri":"8:b"}]},{"key":"follow","users":[{"mri":"8:c"}]},{"key":"heart","users":[{"mri":"8:c"}]}]}"#;
        assert_eq!(reaction_count(json), 3);
        assert_eq!(reaction_count("{}"), 0);
        assert_eq!(reaction_count("not json"), 0);
    }

    #[test]
    fn posts_rank_by_reactions_and_replies_then_by_time() {
        let ranked = rank(vec![
            post("quiet-new", 30, 0, 0),
            post("liked", 10, 3, 0),
            post("discussed", 5, 1, 3),
            post("quiet-old", 20, 0, 0),
        ]);
        let ids: Vec<_> = ranked.iter().map(|p| p.row.id.as_str()).collect();
        assert_eq!(ids, ["discussed", "liked", "quiet-new", "quiet-old"]);
    }

    #[test]
    fn engagement_text_counts_what_there_is() {
        assert_eq!(engagement(0, 0), "");
        assert_eq!(engagement(1, 0), "1 reaction");
        assert_eq!(engagement(2, 1), "2 reactions · 1 reply");
        assert_eq!(engagement(0, 4), "4 replies");
    }
}
