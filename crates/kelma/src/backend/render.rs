//! The render loop: rebuilds the visible view data from the store and pushes it to the UI.

use super::Engine;
use super::util::*;
use crate::data::{
    ChatData, FileData, HeaderData, ImgData, MsgData, Pixels, ReactData, TeamData, color_for,
    initials,
};
use crate::markdown::{reaction_emoji, render_blocks};
use chrono::{Local, TimeZone};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use store::{ConvFilter, ConvKind, MessageRow, UserRow};

impl Engine {
    pub(super) async fn render_loop(self: Arc<Self>) {
        let mut last_typing = String::new();
        loop {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let d = std::mem::take(&mut *self.dirty.lock().unwrap());
            if d.chats {
                self.render_chats();
            }
            if d.teams {
                self.render_teams();
            }
            if d.header {
                self.render_header();
            }
            if d.messages {
                self.render_messages(d.scroll);
            }
            let typing = {
                let mut v = self.view.lock().unwrap();
                let now = Instant::now();
                v.typing.retain(|_, (_, until)| *until > now);
                v.current
                    .as_ref()
                    .and_then(|c| v.typing.get(c))
                    .map(|(n, _)| format!("{n} is typing…"))
                    .unwrap_or_default()
            };
            if typing != last_typing {
                self.ui.typing(typing.clone());
                last_typing = typing;
            }
        }
    }

    fn avatar(self: &Arc<Self>, mri: &str) -> Option<Pixels> {
        if mri.is_empty() || mri.starts_with("28:") {
            return None;
        }
        let mut cache = self.avatars.lock().unwrap();
        if let Some(p) = cache.get(mri) {
            return p.clone();
        }
        cache.insert(mri.to_string(), None);
        drop(cache);
        // Stored avatar first, network second.
        let stored = self
            .store
            .lock()
            .unwrap()
            .user(mri)
            .ok()
            .flatten()
            .and_then(|u| u.avatar);
        if let Some(bytes) = stored {
            let px = decode_image(&bytes, 64, 64);
            self.avatars
                .lock()
                .unwrap()
                .insert(mri.to_string(), px.clone());
            return px;
        }
        let e = self.clone();
        let mri = mri.to_string();
        tokio::spawn(async move {
            if let Ok(bytes) = e.client.avatar(&e.me, &mri).await
                && let Some(px) = decode_image(&bytes, 64, 64)
            {
                e.avatars.lock().unwrap().insert(mri.clone(), Some(px));
                let st = e.store.lock().unwrap();
                let mut u = st.user(&mri).ok().flatten().unwrap_or(UserRow {
                    mri: mri.clone(),
                    display_name: String::new(),
                    email: None,
                    avatar: None,
                    updated_ms: 0,
                });
                u.avatar = Some(bytes);
                let _ = st.upsert_user(&u);
                drop(st);
                e.mark(|d| {
                    d.chats = true;
                    d.messages = true;
                    d.header = true;
                });
            }
        });
        None
    }

    fn inline_image(self: &Arc<Self>, url: &str) -> Option<Pixels> {
        let mut cache = self.images.lock().unwrap();
        if let Some(p) = cache.get(url) {
            return p.clone();
        }
        cache.insert(url.to_string(), None);
        drop(cache);
        let e = self.clone();
        let url = url.to_string();
        tokio::spawn(async move {
            let bytes = if teams_proto::ams::is_ams_url(&url) {
                e.client.download_image(&url).await
            } else {
                e.client.bytes(&url, &[], 10 * 1024 * 1024).await
            };
            match bytes {
                Ok(bytes) => {
                    match tokio::task::spawn_blocking(move || decode_image(&bytes, 480, 360))
                        .await
                        .ok()
                        .flatten()
                    {
                        Some(px) => {
                            e.images.lock().unwrap().insert(url, Some(px));
                            e.mark(|d| d.messages = true);
                        }
                        None => tracing::warn!(%url, "image could not be decoded"),
                    }
                }
                Err(err) => {
                    tracing::warn!(%url, %err, "image download failed, retrying in 30s");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    e.images.lock().unwrap().remove(&url);
                    e.mark(|d| d.messages = true);
                }
            }
        });
        None
    }

    fn other_member(&self, conv_id: &str) -> Option<String> {
        one_on_one_members(conv_id)
            .into_iter()
            .find(|m| m != &self.me)
    }

    fn render_chats(self: &Arc<Self>) {
        let (filter, search, current) = {
            let v = self.view.lock().unwrap();
            (v.filter.clone(), v.search.clone(), v.current.clone())
        };
        let rows = {
            let st = self.store.lock().unwrap();
            if search.trim().is_empty() {
                let f = match filter.as_str() {
                    "unread" => ConvFilter::Unread,
                    "channels" => ConvFilter::Channels,
                    "chats" => ConvFilter::Chats,
                    _ => ConvFilter::All,
                };
                st.conversations(f, 300).unwrap_or_default()
            } else {
                st.search_conversations(search.trim(), 100)
                    .unwrap_or_default()
            }
        };
        let mut fav = vec![];
        let mut main = vec![];
        let mut chans = vec![];
        for r in rows {
            if r.kind == ConvKind::Channel && r.last_activity_ms == 0 {
                continue;
            }
            let avatar_mri = match r.kind {
                ConvKind::OneOnOne => self.other_member(&r.id),
                ConvKind::Notes => Some(self.me.clone()),
                _ => None,
            };
            let title = match (&r.kind, &r.team_name) {
                (ConvKind::Channel, Some(t)) => format!("{} › {}", t, r.title),
                _ => r.title.clone(),
            };
            let data = ChatData {
                initials: initials(&r.title),
                color: color_for(&r.id),
                avatar: avatar_mri.and_then(|m| self.avatar(&m)),
                rtl_title: render::base_direction(&title) == render::Dir::Rtl,
                rtl_preview: render::base_direction(&r.last_preview) == render::Dir::Rtl,
                preview: r.last_preview.chars().take(120).collect(),
                time: chat_time(r.last_activity_ms),
                unread: r.unread && current.as_deref() != Some(r.id.as_str()),
                favorite: r.favorite,
                section: if r.favorite {
                    "favorites"
                } else if r.kind == ConvKind::Channel {
                    "channels"
                } else {
                    "chats"
                },
                id: r.id,
                title,
            };
            match data.section {
                "favorites" => fav.push(data),
                "channels" => chans.push(data),
                _ => main.push(data),
            }
        }
        self.ui.chats(fav, main, chans);
    }

    fn render_teams(self: &Arc<Self>) {
        let (order, collapsed) = {
            let v = self.view.lock().unwrap();
            (v.teams_order.clone(), v.collapsed_teams.clone())
        };
        let st = self.store.lock().unwrap();
        let mut teams = st.teams().unwrap_or_default();
        teams.sort_by_key(|t| order.iter().position(|o| o == &t.id).unwrap_or(usize::MAX));
        // ToggleTeam(index) refers to this order.
        self.view.lock().unwrap().teams_order = teams.iter().map(|t| t.id.clone()).collect();
        let data: Vec<TeamData> = teams
            .into_iter()
            .map(|t| {
                let mut channels = st.channels(&t.id).unwrap_or_default();
                channels.sort_by_key(|c| (c.title != "General", c.title.to_lowercase()));
                TeamData {
                    initials: initials(&t.name),
                    color: color_for(&t.id),
                    expanded: !collapsed.contains(&t.id),
                    channels: channels
                        .into_iter()
                        .map(|c| (c.id, c.title, c.unread))
                        .collect(),
                    id: t.id,
                    name: t.name,
                }
            })
            .collect();
        drop(st);
        self.ui.teams(data);
    }

    fn render_header(self: &Arc<Self>) {
        let (cur, thread, title_override) = {
            let v = self.view.lock().unwrap();
            (
                v.current.clone(),
                v.thread_root.clone(),
                v.current_title.clone(),
            )
        };
        let Some(id) = cur else {
            self.ui.header(HeaderData::default());
            return;
        };
        let row = self.store.lock().unwrap().conversation(&id).ok().flatten();
        let (title, kind) = match &row {
            Some(r) => (
                if title_override.is_empty() {
                    r.title.clone()
                } else {
                    title_override.clone()
                },
                r.kind,
            ),
            None => (title_override.clone(), ConvKind::Group),
        };
        let avatar_mri = match kind {
            ConvKind::OneOnOne => self.other_member(&id),
            ConvKind::Notes => Some(self.me.clone()),
            _ => None,
        };
        self.ui.header(HeaderData {
            initials: initials(&title),
            color: color_for(&id),
            avatar: avatar_mri.and_then(|m| self.avatar(&m)),
            is_channel: kind == ConvKind::Channel,
            thread_root: thread.unwrap_or_default(),
            id,
            title,
        });
    }

    fn render_messages(self: &Arc<Self>, scroll: bool) {
        let (cur, thread, limit, pending) = {
            let v = self.view.lock().unwrap();
            (
                v.current.clone(),
                v.thread_root.clone(),
                v.limit,
                v.pending.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
            )
        };
        let Some(conv) = cur else {
            self.ui.messages(vec![], false);
            return;
        };
        let (rows, is_channel, reply_counts) = {
            let st = self.store.lock().unwrap();
            let is_channel = st
                .conversation(&conv)
                .ok()
                .flatten()
                .is_some_and(|c| c.kind == ConvKind::Channel);
            let mut rows = match &thread {
                Some(root) => {
                    let mut r: Vec<MessageRow> =
                        st.message(root).ok().flatten().into_iter().collect();
                    r.extend(st.thread_replies(root, 500).unwrap_or_default());
                    r
                }
                None => st.messages_before(&conv, None, limit).unwrap_or_default(),
            };
            rows.extend(
                pending
                    .into_iter()
                    .filter(|p| p.conversation_id == conv && p.parent_id == thread),
            );
            let counts: HashMap<String, usize> = if is_channel && thread.is_none() {
                rows.iter()
                    .map(|r| (r.id.clone(), st.reply_count(&r.id).unwrap_or(0)))
                    .collect()
            } else {
                HashMap::new()
            };
            (rows, is_channel, counts)
        };
        let mut out: Vec<MsgData> = Vec::with_capacity(rows.len());
        let mut prev: Option<&MessageRow> = None;
        for r in rows
            .iter()
            .filter(|r| !r.message_type.starts_with("Control/"))
        {
            let system = system_text(r);
            let props: Value =
                serde_json::from_str(&r.reactions_json).unwrap_or_else(|_| json!({}));
            let parsed = render_blocks(&render::parse_teams_html(&r.content_html));
            let mine = r.sender_mri == self.me;
            let new_day = prev.is_none_or(|p| !same_day(p.compose_ms, r.compose_ms));
            let show_header = new_day
                || prev.is_none_or(|p| {
                    p.sender_mri != r.sender_mri
                        || r.compose_ms - p.compose_ms > 5 * 60_000
                        || system_text(p).is_some()
                });
            let reactions = props
                .get("r")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| {
                            let key = x.get("key")?.as_str()?.to_string();
                            let users = x.get("users")?.as_array()?;
                            Some(ReactData {
                                emoji: reaction_emoji(&key),
                                count: users.len() as i32,
                                mine: users.iter().any(|u| u == self.me.as_str()),
                                key,
                            })
                        })
                        .filter(|r| r.count > 0)
                        .collect()
                })
                .unwrap_or_default();
            let files = props
                .get("f")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|f| {
                            let name = f.get("name")?.as_str()?.to_string();
                            let ext = f
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_uppercase();
                            Some(FileData {
                                url: f.get("url")?.as_str()?.to_string(),
                                ext: if ext.is_empty() { "FILE".into() } else { ext },
                                name,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let images = parsed
                .images
                .iter()
                .map(|(url, w, h)| {
                    let pixels = self.inline_image(url);
                    // Real pixel size once decoded; the HTML attributes are only a hint.
                    let (w, h) = match &pixels {
                        Some(p) => (p.width() as f32, p.height() as f32),
                        None => (w.unwrap_or(320) as f32, h.unwrap_or(200) as f32),
                    };
                    let scale = (420.0 / w.max(1.0)).min(320.0 / h.max(1.0)).min(1.0);
                    ImgData {
                        pixels,
                        url: url.clone(),
                        w: w * scale,
                        h: h * scale,
                    }
                })
                .collect();
            let sender = if r.sender_name.is_empty() {
                self.store
                    .lock()
                    .unwrap()
                    .user(&r.sender_mri)
                    .ok()
                    .flatten()
                    .map(|u| u.display_name)
                    .unwrap_or_default()
            } else {
                r.sender_name.clone()
            };
            out.push(MsgData {
                id: r.id.clone(),
                initials: initials(&sender),
                color: color_for(&r.sender_mri),
                avatar: if show_header && !mine {
                    self.avatar(&r.sender_mri)
                } else {
                    None
                },
                sender,
                mine,
                time: Local
                    .timestamp_millis_opt(r.compose_ms)
                    .single()
                    .map(|t| t.format("%H:%M").to_string())
                    .unwrap_or_default(),
                day: if new_day {
                    day_label(r.compose_ms)
                } else {
                    String::new()
                },
                show_header,
                edited: r.edited_ms.is_some(),
                deleted: r.deleted,
                plain: system.clone().unwrap_or(parsed.plain),
                system: system.is_some(),
                subject: props
                    .get("s")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                quote_author: parsed.quote_author,
                quote_text: parsed.quote_text,
                markdown: parsed.markdown,
                rtl: parsed.rtl,
                images,
                files,
                reactions,
                reply_count: if is_channel && thread.is_none() {
                    reply_counts.get(&r.id).copied().unwrap_or(0) as i32
                } else {
                    -1
                },
                pending: r.id.starts_with("pending-"),
            });
            prev = Some(r);
        }
        self.ui.messages(out, scroll);
    }
}
