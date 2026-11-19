//! Channel posts like Teams (plan 8.4), files attached from the cloud (8.7) and structured
//! Adaptive Cards (8.8).
//!
//! The Posts tab shows each post as a card: the post, its first two replies and "See N more
//! replies". Replies come from the store, which holds the thread once it was opened or came in
//! live; "See more" fetches the whole thread. Everything else reuses the chat code: replies go
//! through the outbox like any message.

use super::Engine;
use super::outbox::row_from_outbox;
use super::render::MsgMarks;
use crate::data::{MsgData, Pixels};
use crate::lists_data::{CardCellData, CardRowData, PostData};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use store::{MessageRow, OutboxKind};
use teams_proto::chat_card::{BANNER_THEMES, Band, Kind, Row, Tone, rows_for_attachments};
use teams_proto::delivery::{Announcement as Banner, Delivery};

/// Replies shown under a post before "See N more replies".
const REPLIES_SHOWN: usize = 2;
/// Posts whose threads are fetched when a channel opens, so their cards show the replies.
const PREFETCH_POSTS: usize = 12;
/// Key of the "See new posts at top" choice in the store's key/value table.
const KV_NEWEST_TOP: &str = "posts_newest_top";

/// Commands of the Posts tab (`ChannelStore.command(name, argument)`).
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelCmd {
    Expand(String),
    /// Post id and the reply text.
    Reply(String, String),
    Follow(String, bool),
    /// A message id of a post or a reply (from the message menu).
    FollowMessage(String),
    /// `true`: newest posts on top.
    Order(bool),
    Announce(Announcement),
}

/// What the composer sends for "Announcement".
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    pub subject: String,
    pub headline: String,
    pub theme: String,
    pub text: String,
}

impl ChannelCmd {
    pub fn parse(name: &str, arg: &str) -> Option<ChannelCmd> {
        let (a, b) = arg.split_once('\t').unwrap_or((arg, ""));
        Some(match name {
            "expand" => ChannelCmd::Expand(a.into()),
            "reply" => ChannelCmd::Reply(a.into(), b.into()),
            "follow" => ChannelCmd::Follow(a.into(), true),
            "unfollow" => ChannelCmd::Follow(a.into(), false),
            "follow-msg" => ChannelCmd::FollowMessage(a.into()),
            "order" => ChannelCmd::Order(a == "top"),
            "announce" => {
                let mut p = arg.splitn(4, '\t');
                ChannelCmd::Announce(Announcement {
                    subject: p.next()?.into(),
                    headline: p.next()?.into(),
                    theme: p.next()?.into(),
                    text: p.next()?.into(),
                })
            }
            _ => return None,
        })
    }
}

/// Engine state of the Posts tab.
#[derive(Default)]
pub(super) struct State {
    /// Posts whose replies are all shown.
    expanded: HashSet<String>,
    /// Threads followed or unfollowed in this session (Teams does not tell us at start).
    followed: HashMap<String, bool>,
    newest_top: Option<bool>,
}

fn tone(t: Tone) -> &'static str {
    match t {
        Tone::Default => "default",
        Tone::Dark => "dark",
        Tone::Light => "light",
        Tone::Accent => "accent",
        Tone::Good => "good",
        Tone::Warning => "warning",
        Tone::Attention => "attention",
    }
}

fn band(b: Band) -> &'static str {
    match b {
        Band::None => "",
        Band::Emphasis => "emphasis",
        Band::Good => "good",
        Band::Attention => "attention",
        Band::Warning => "warning",
        Band::Accent => "accent",
    }
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Text => "text",
        Kind::Fact => "fact",
        Kind::Columns => "columns",
        Kind::Image => "image",
        Kind::Actions => "actions",
        Kind::Banner => "banner",
    }
}

/// Size of a card image before its pixels are known, then its real size capped for the card.
fn image_size(px: &Option<Pixels>) -> (f32, f32) {
    match px {
        Some(p) => {
            let (w, h) = (p.width() as f32, p.height() as f32);
            let scale = (360.0 / w.max(1.0)).min(240.0 / h.max(1.0)).min(1.0);
            (w * scale, h * scale)
        }
        None => (120.0, 80.0),
    }
}

impl Engine {
    /// The drawn rows of a message's card (`props` is the stored message extras), or none.
    pub(super) fn card_rows(self: &Arc<Self>, props: &Value, headline: &str) -> Vec<CardRowData> {
        let Some(raw) = props.get("c").and_then(Value::as_str) else {
            return Vec::new();
        };
        let Ok(attachments) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        rows_for_attachments(&attachments, headline)
            .into_iter()
            .map(|r| self.card_row(r))
            .collect()
    }

    fn card_row(self: &Arc<Self>, r: Row) -> CardRowData {
        let pixels = (r.kind == Kind::Image && !r.image.is_empty())
            .then(|| self.inline_image(&r.image))
            .flatten();
        let (image_w, image_h) = if r.kind == Kind::Image {
            image_size(&pixels)
        } else {
            (0.0, 0.0)
        };
        CardRowData {
            kind: kind(r.kind),
            // Markdown has no font weight, so bold text is marked up.
            text: if r.bold && r.kind == Kind::Text && !r.text.contains("**") {
                format!("**{}**", r.text)
            } else {
                r.text
            },
            label: r.label,
            size: f32::from(r.size),
            bold: r.bold,
            subtle: r.subtle,
            tone: tone(r.tone),
            band: band(r.band),
            cells: r
                .cells
                .into_iter()
                .map(|c| CardCellData {
                    pixels: c.image.as_deref().and_then(|u| self.inline_image(u)),
                    text: if c.bold && !c.text.contains("**") {
                        format!("**{}**", c.text)
                    } else {
                        c.text
                    },
                    stretch: c.stretch,
                    size: f32::from(c.size),
                    bold: c.bold,
                    tone: tone(c.tone),
                })
                .collect(),
            buttons: r
                .buttons
                .into_iter()
                .map(|b| (b.title, b.url.unwrap_or_default()))
                .collect(),
            pixels,
            image_w,
            image_h,
        }
    }

    fn newest_top(&self) -> bool {
        let mut st = self.channels.lock().unwrap();
        *st.newest_top.get_or_insert_with(|| {
            self.store
                .lock()
                .unwrap()
                .get_kv(KV_NEWEST_TOP)
                .ok()
                .flatten()
                .as_deref()
                == Some("1")
        })
    }

    /// Builds and pushes the post cards of the open channel. `rows` are the root posts and
    /// `out` their view data, in the same order.
    pub(super) fn render_posts(self: &Arc<Self>, conv: &str, rows: &[MessageRow], out: &[MsgData]) {
        let newest_top = self.newest_top();
        let (expanded, followed) = {
            let st = self.channels.lock().unwrap();
            (st.expanded.clone(), st.followed.clone())
        };
        let roots: Vec<&MessageRow> = rows
            .iter()
            .filter(|r| !r.message_type.starts_with("Control/"))
            .collect();
        let mut posts = Vec::with_capacity(roots.len());
        for (root, data) in roots.iter().zip(out) {
            let (replies, outbox) = {
                let st = self.store.lock().unwrap();
                let replies = st.thread_replies(&root.id, 500).unwrap_or_default();
                let target = format!("{conv};messageid={}", root.id);
                let outbox: Vec<MessageRow> = st
                    .outbox_for(&target)
                    .unwrap_or_default()
                    .iter()
                    .map(|i| row_from_outbox(i, &self.me, &self.my_name))
                    .collect();
                (replies, outbox)
            };
            let open = expanded.contains(&root.id);
            let total = replies.len();
            let shown = if open {
                total
            } else {
                total.min(REPLIES_SHOWN)
            };
            let mut visible: Vec<MessageRow> = replies.into_iter().take(shown).collect();
            visible.extend(outbox);
            let reply_data = self.build_msgs(
                &visible,
                false,
                &Some(root.id.clone()),
                &HashMap::new(),
                &MsgMarks::default(),
            );
            // Inside a card there are no day dividers, and every message shows who wrote it.
            let mut post = data.clone();
            post.day = String::new();
            post.show_header = true;
            let reply_data = reply_data
                .into_iter()
                .map(|mut r| {
                    r.day = String::new();
                    r.show_header = true;
                    r
                })
                .collect();
            posts.push(PostData {
                post,
                replies: reply_data,
                more: (total - shown) as i32,
                expanded: open,
                following: followed.get(&root.id).copied().unwrap_or(false),
            });
        }
        if newest_top {
            posts.reverse();
        }
        self.ui.posts(posts, newest_top);
    }

    /// Fetches the threads of the newest posts of a channel in the background (the history page
    /// only holds some replies), so the post cards show them.
    pub(super) fn prefetch_threads(self: &Arc<Self>, conv: &str) {
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let roots: Vec<String> = {
            let st = self.store.lock().unwrap();
            if !st
                .conversation(conv)
                .ok()
                .flatten()
                .is_some_and(|c| c.kind == store::ConvKind::Channel)
            {
                return;
            }
            st.messages_before(conv, None, PREFETCH_POSTS)
                .unwrap_or_default()
                .into_iter()
                .rev()
                .map(|r| r.id)
                .collect()
        };
        let (e, conv) = (self.clone(), conv.to_string());
        tokio::spawn(async move {
            for root in roots {
                // Stop when the user left the channel.
                if e.view.lock().unwrap().current.as_deref() != Some(conv.as_str()) {
                    break;
                }
                let thread = format!("{conv};messageid={root}");
                if let Ok(page) = e.client.messages(&thread, None, 50).await {
                    let replies: Vec<_> =
                        page.messages.into_iter().filter(|m| m.id != root).collect();
                    e.store_messages(&replies, Some(&root));
                    e.mark(|d| d.messages = true);
                }
            }
        });
    }

    pub(super) async fn channel_cmd(self: &Arc<Self>, cmd: ChannelCmd) -> anyhow::Result<()> {
        match cmd {
            ChannelCmd::Expand(root) => self.expand_post(&root).await,
            ChannelCmd::Reply(root, text) => self.reply_in_thread(&root, &text).await,
            ChannelCmd::Follow(root, on) => self.follow_thread(&root, on).await,
            ChannelCmd::FollowMessage(id) => {
                let Some(row) = self.store.lock().unwrap().message(&id)? else {
                    return Ok(());
                };
                let root = row.parent_id.unwrap_or(row.id);
                let on = !self
                    .channels
                    .lock()
                    .unwrap()
                    .followed
                    .get(&root)
                    .copied()
                    .unwrap_or(false);
                self.follow_thread(&root, on).await
            }
            ChannelCmd::Order(top) => {
                self.channels.lock().unwrap().newest_top = Some(top);
                self.store
                    .lock()
                    .unwrap()
                    .set_kv(KV_NEWEST_TOP, if top { "1" } else { "0" })?;
                self.mark(|d| {
                    d.messages = true;
                    d.scroll = true;
                });
                Ok(())
            }
            ChannelCmd::Announce(a) => self.announce(a).await,
        }
    }

    /// "See N more replies": show the whole thread, fetching what is not stored yet.
    async fn expand_post(self: &Arc<Self>, root: &str) -> anyhow::Result<()> {
        self.channels
            .lock()
            .unwrap()
            .expanded
            .insert(root.to_string());
        self.mark(|d| d.messages = true);
        let conv = self.view.lock().unwrap().current.clone();
        if let Some(c) = conv
            && !self.offline.load(std::sync::atomic::Ordering::Relaxed)
        {
            let page = self
                .client
                .messages(&format!("{c};messageid={root}"), None, 100)
                .await?;
            self.store_messages(
                &page
                    .messages
                    .into_iter()
                    .filter(|m| m.id != root)
                    .collect::<Vec<_>>(),
                Some(root),
            );
            self.mark(|d| d.messages = true);
        }
        Ok(())
    }

    /// The inline reply box: queue the text as a reply in the post's thread.
    async fn reply_in_thread(self: &Arc<Self>, root: &str, text: &str) -> anyhow::Result<()> {
        let text = text.trim();
        let conv = self.view.lock().unwrap().current.clone();
        let (Some(conv), false) = (conv, text.is_empty()) else {
            return Ok(());
        };
        let html = render::text_to_teams_html(text);
        let item = self.new_outbox_item(
            format!("{conv};messageid={root}"),
            OutboxKind::Text,
            html,
            None,
            None,
            None,
        );
        self.enqueue(item)?;
        self.flush_outbox(false).await;
        self.mark(|d| d.messages = true);
        Ok(())
    }

    async fn follow_thread(self: &Arc<Self>, root: &str, on: bool) -> anyhow::Result<()> {
        let Some(conv) = self.view.lock().unwrap().current.clone() else {
            return Ok(());
        };
        let was = self
            .channels
            .lock()
            .unwrap()
            .followed
            .insert(root.to_string(), on);
        self.mark(|d| d.messages = true);
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        if let Err(err) = self.client.set_thread_followed(&conv, root, on).await {
            let mut st = self.channels.lock().unwrap();
            match was {
                Some(w) => st.followed.insert(root.to_string(), w),
                None => st.followed.remove(root),
            };
            drop(st);
            self.mark(|d| d.messages = true);
            return Err(err.into());
        }
        self.ui.banner(
            if on {
                "You follow this thread"
            } else {
                "You no longer follow this thread"
            }
            .into(),
        );
        Ok(())
    }

    /// Sends an announcement post (a headline and a colored banner above the post).
    async fn announce(self: &Arc<Self>, a: Announcement) -> anyhow::Result<()> {
        let conv = self.view.lock().unwrap().current.clone();
        let Some(conv) = conv else {
            return Ok(());
        };
        if a.headline.trim().is_empty() {
            self.ui
                .banner("Type a headline for the announcement".into());
            return Ok(());
        }
        if a.text.trim().is_empty() {
            self.ui.banner("Write something under the headline".into());
            return Ok(());
        }
        self.ui.clear_subject();
        let html = render::text_to_teams_html(a.text.trim());
        let theme = BANNER_THEMES
            .iter()
            .find(|(n, _)| *n == a.theme)
            .map_or(0, |(_, i)| *i);
        let announcement = Banner::with_theme(a.headline.trim(), theme)?;
        let subject = a.subject.trim();
        self.client
            .send_message_delivery(
                &conv,
                &html,
                &self.my_name,
                (!subject.is_empty()).then_some(subject),
                &Delivery::Announcement(announcement),
            )
            .await?;
        self.fetch_latest(&conv).await;
        self.mark(|d| {
            d.messages = true;
            d.scroll = true;
        });
        Ok(())
    }

    // ---- attach cloud files (8.7) ----

    /// A file picked in the OneDrive picker ("Attach cloud files") is posted to the open chat as a
    /// file card. In a chat the other members get access first.
    pub(super) async fn attach_cloud_file(
        self: &Arc<Self>,
        drive_id: &str,
        item_id: &str,
        name: &str,
    ) -> anyhow::Result<()> {
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            self.ui
                .banner(format!("Demo mode: {name} would be attached"));
            return Ok(());
        }
        self.ui.banner(format!("Attaching {name}…"));
        let cmid = teams_proto::time::now_ms().to_string();
        let sent = self
            .client
            .send_cloud_file_as(&target, drive_id, item_id, &self.my_name, &cmid)
            .await;
        self.ui.banner(String::new());
        sent?;
        let conv = self.view.lock().unwrap().current.clone();
        if let Some(conv) = conv {
            self.fetch_latest(&conv).await;
        }
        self.mark(|d| {
            d.chats = true;
            d.messages = true;
            d.scroll = true;
        });
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_followed(&self, root: &str) -> Option<bool> {
        self.channels.lock().unwrap().followed.get(root).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(
            ChannelCmd::parse("reply", "1700\thello there"),
            Some(ChannelCmd::Reply("1700".into(), "hello there".into()))
        );
        assert_eq!(
            ChannelCmd::parse("unfollow", "1700"),
            Some(ChannelCmd::Follow("1700".into(), false))
        );
        assert_eq!(
            ChannelCmd::parse("order", "top"),
            Some(ChannelCmd::Order(true))
        );
        assert_eq!(
            ChannelCmd::parse("order", "bottom"),
            Some(ChannelCmd::Order(false))
        );
        // The text of an announcement may hold tabs: only the first three splits count.
        assert_eq!(
            ChannelCmd::parse("announce", "Subject\tBig news\tgreen\tline\twith tab"),
            Some(ChannelCmd::Announce(Announcement {
                subject: "Subject".into(),
                headline: "Big news".into(),
                theme: "green".into(),
                text: "line\twith tab".into(),
            }))
        );
        assert_eq!(ChannelCmd::parse("announce", "only"), None);
        assert_eq!(ChannelCmd::parse("nope", ""), None);
    }

    #[test]
    fn card_enums_map_to_the_names_the_ui_uses() {
        assert_eq!(kind(Kind::Columns), "columns");
        assert_eq!(band(Band::None), "");
        assert_eq!(band(Band::Attention), "attention");
        assert_eq!(tone(Tone::Warning), "warning");
    }

    #[test]
    fn card_images_start_small_and_are_capped_when_decoded() {
        assert_eq!(image_size(&None), (120.0, 80.0));
        let big = Pixels::new(720, 480);
        let (w, h) = image_size(&Some(big));
        assert!(w <= 360.0 && h <= 240.0, "{w}x{h}");
    }
}
