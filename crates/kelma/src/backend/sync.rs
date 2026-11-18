//! Initial sync, conversation history fetching and store writes.

use super::util::*;
use super::{Engine, PAGE};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use store::{ConvKind, ConversationRow, TeamRow, UserRow};
use teams_proto::model::Message;
use teams_proto::time::now_ms;

impl Engine {
    /// Fetches conversations, teams and names; writes them to the store.
    pub async fn sync(self: &Arc<Self>) -> anyhow::Result<()> {
        let convs = self.client.conversations(0, 200).await?;
        let snap = self.client.snapshot().await.unwrap_or_default();
        let meta: HashMap<&str, &teams_proto::csa::ChatMeta> =
            snap.chats.iter().map(|c| (c.id.as_str(), c)).collect();
        let mut channel_info: HashMap<String, (String, String, String)> = HashMap::new();
        for t in &snap.teams {
            for ch in &t.channels {
                channel_info.insert(
                    ch.id.clone(),
                    (t.id.clone(), t.name.clone(), ch.name.clone()),
                );
            }
        }

        // Resolve names we need for titles.
        let mut wanted: Vec<String> = vec![];
        for c in &convs {
            let members = meta
                .get(c.id.as_str())
                .map(|m| m.members.clone())
                .unwrap_or_else(|| one_on_one_members(&c.id));
            wanted.extend(members.into_iter().filter(|m| m != &self.me));
        }
        wanted.sort();
        wanted.dedup();
        let missing = self.store.lock().unwrap().users_missing_profile(&wanted)?;
        if !missing.is_empty()
            && let Ok(profiles) = self.client.profiles(&missing).await
        {
            let st = self.store.lock().unwrap();
            for p in profiles {
                st.upsert_user(&UserRow {
                    mri: p.mri,
                    display_name: p.display_name,
                    email: p.email,
                    avatar: None,
                    updated_ms: now_ms(),
                })?;
            }
        }

        {
            let st = self.store.lock().unwrap();
            for t in &snap.teams {
                st.upsert_team(&TeamRow {
                    id: t.id.clone(),
                    name: t.name.clone(),
                })?;
            }
            let mut seen = HashSet::new();
            for c in &convs {
                let Some(kind) = kind_from(c.kind) else {
                    continue;
                };
                if meta.get(c.id.as_str()).is_some_and(|m| m.hidden) {
                    continue;
                }
                seen.insert(c.id.clone());
                let name_of = |mri: &str| st.user(mri).ok().flatten().map(|u| u.display_name);
                let members = meta
                    .get(c.id.as_str())
                    .map(|m| m.members.clone())
                    .unwrap_or_else(|| one_on_one_members(&c.id));
                let others: Vec<String> = members
                    .iter()
                    .filter(|m| *m != &self.me)
                    .filter_map(|m| name_of(m))
                    .collect();
                let (team_id, team_name, channel_name) = channel_info
                    .get(&c.id)
                    .cloned()
                    .map(|(a, b, cn)| (Some(a), Some(b), Some(cn)))
                    .unwrap_or((None, None, None));
                let title = match kind {
                    ConvKind::Notes => format!("{} (You)", self.my_name),
                    ConvKind::Channel => channel_name
                        .unwrap_or_else(|| c.topic.clone().unwrap_or_else(|| "Channel".into())),
                    _ => meta
                        .get(c.id.as_str())
                        .and_then(|m| m.title.clone())
                        .or_else(|| c.topic.clone())
                        .or_else(|| {
                            (!others.is_empty()).then(|| {
                                others
                                    .iter()
                                    .take(4)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                        })
                        .unwrap_or_else(|| "Chat".into()),
                };
                let last = c
                    .last_message
                    .as_ref()
                    .filter(|m| m.is_chat_content() || m.message_type.starts_with("RichText"));
                let preview = last.map(|m| preview_of(m, &self.me)).unwrap_or_default();
                let unread = last
                    .is_some_and(|m| m.sender_mri != self.me && m.compose_ms > c.read_horizon_ms);
                let existing = st.conversation(&c.id)?;
                st.upsert_conversation(&ConversationRow {
                    id: c.id.clone(),
                    kind,
                    title,
                    team_id: team_id.or_else(|| existing.as_ref().and_then(|e| e.team_id.clone())),
                    team_name: team_name
                        .or_else(|| existing.as_ref().and_then(|e| e.team_name.clone())),
                    last_activity_ms: c.last_activity_ms,
                    last_preview: preview,
                    unread,
                    favorite: false,
                    read_horizon_ms: c.read_horizon_ms,
                })?;
            }
            // Channels that had no recent activity still belong in the Teams view.
            for (id, (tid, tname, cname)) in &channel_info {
                if !seen.contains(id) && st.conversation(id)?.is_none() {
                    st.upsert_conversation(&ConversationRow {
                        id: id.clone(),
                        kind: ConvKind::Channel,
                        title: cname.clone(),
                        team_id: Some(tid.clone()),
                        team_name: Some(tname.clone()),
                        last_activity_ms: 0,
                        last_preview: String::new(),
                        unread: false,
                        favorite: false,
                        read_horizon_ms: 0,
                    })?;
                }
            }
        }
        {
            let mut v = self.view.lock().unwrap();
            v.teams_order = snap.teams.iter().map(|t| t.id.clone()).collect();
        }
        self.mark(|d| {
            d.chats = true;
            d.teams = true;
        });
        // If a conversation is open, refresh its newest page.
        let cur = self.view.lock().unwrap().current.clone();
        if let Some(id) = cur {
            self.fetch_latest(&id).await;
        }
        Ok(())
    }

    pub(super) async fn fetch_latest(self: &Arc<Self>, conv: &str) {
        match self.client.messages(conv, None, PAGE as u32).await {
            Ok(page) => {
                self.store_messages(&page.messages, None);
                self.mark(|d| d.messages = true);
            }
            Err(err) => tracing::warn!(%err, "history fetch failed"),
        }
    }

    pub(super) fn store_messages(&self, msgs: &[Message], parent: Option<&str>) {
        let st = self.store.lock().unwrap();
        for m in msgs {
            if m.message_type.starts_with("Control/") {
                continue;
            }
            let _ = st.upsert_message(&row_from_message(m, parent));
        }
    }

    pub(super) async fn open(
        self: &Arc<Self>,
        id: String,
        title: Option<String>,
    ) -> anyhow::Result<()> {
        {
            let mut v = self.view.lock().unwrap();
            if v.current.as_deref() == Some(id.as_str()) && v.thread_root.is_none() {
                return Ok(());
            }
            v.current = Some(id.clone());
            v.current_title = title.unwrap_or_default();
            v.thread_root = None;
            v.limit = PAGE;
            v.reply_to = None;
            v.editing = None;
        }
        self.ui.reply_bar(String::new(), String::new(), false);
        self.mark(|d| {
            d.header = true;
            d.messages = true;
            d.scroll = true;
            d.chats = true;
        });
        let page = self.client.messages(&id, None, PAGE as u32).await?;
        self.store_messages(&page.messages, None);
        if let Some(last) = page.messages.iter().rev().find(|m| m.is_chat_content()) {
            self.store.lock().unwrap().mark_read(&id, last.compose_ms)?;
            let _ = self.client.mark_read(&id, last).await;
        }
        self.mark(|d| {
            d.messages = true;
            d.scroll = true;
            d.chats = true;
        });
        Ok(())
    }

    pub(super) async fn load_older(self: &Arc<Self>) -> anyhow::Result<()> {
        let (conv, thread, limit) = {
            let v = self.view.lock().unwrap();
            (v.current.clone(), v.thread_root.clone(), v.limit)
        };
        let Some(conv) = conv else { return Ok(()) };
        if thread.is_some() {
            return Ok(());
        }
        let cached = self
            .store
            .lock()
            .unwrap()
            .messages_before(&conv, None, limit + PAGE)?;
        if cached.len() < limit + PAGE {
            let oldest = cached.first().map(|m| m.compose_ms);
            if let Some(oldest) = oldest {
                self.ui.loading_older(true);
                let page = self.client.messages(&conv, Some(oldest), PAGE as u32).await;
                self.ui.loading_older(false);
                if let Ok(page) = page {
                    if page.messages.is_empty() {
                        return Ok(());
                    }
                    self.store_messages(&page.messages, None);
                }
            }
        }
        self.view.lock().unwrap().limit = limit + PAGE;
        self.mark(|d| d.messages = true);
        Ok(())
    }
}
