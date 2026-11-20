//! Initial sync, conversation history fetching and store writes.

use super::util::*;
use super::{Engine, PAGE};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use store::{ConvFilter, ConvKind, ConversationRow, TeamRow, UserRow};
use teams_proto::model::Message;
use teams_proto::time::now_ms;

impl Engine {
    /// Fetches conversations, teams and names; writes them to the store.
    pub async fn sync(self: &Arc<Self>) -> anyhow::Result<()> {
        let convs = self.client.conversations(0, 200).await?;
        let snap = self.client.snapshot().await.unwrap_or_default();
        self.remember_members(&snap.chats);
        self.lists_note_chats(&snap.chats);
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

        // With the teams read, a channel they no longer list was deleted (or is in a team the
        // user left): it leaves the Teams view instead of showing as "Channel".
        let teams_known = !snap.teams.is_empty();

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
                if !channel_listed(kind, teams_known, channel_info.contains_key(&c.id)) {
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
                // A manual "Mark as unread" is kept here; the server horizon does not know it.
                let manual = st.unread_mark(&c.id)?;
                // "Mark as unread" from another Teams client arrives as the server bookmark.
                let manual = manual.or(c.unread_bookmark);
                let unread = manual.is_some()
                    || last.is_some_and(|m| {
                        m.sender_mri != self.me && m.compose_ms > c.read_horizon_ms
                    });
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
                    read_horizon_ms: manual.unwrap_or(c.read_horizon_ms),
                })?;
            }
            if teams_known {
                for c in st.conversations(ConvFilter::Channels, 100_000)? {
                    if !channel_info.contains_key(&c.id) {
                        st.delete_conversation(&c.id)?;
                    }
                }
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
        // Sections, channel visibility and notification state come from other aggregator calls.
        let e = self.clone();
        tokio::spawn(async move { e.lists_refresh().await });
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
            if m.sender_mri == self.me
                && let Some(cmid) = &m.client_message_id
            {
                // History shows a message we sent: it no longer waits in the outbox.
                let _ = st.remove(cmid);
                self.view.lock().unwrap().pending.retain(|(c, _)| c != cmid);
            }
        }
    }

    pub(super) async fn open(
        self: &Arc<Self>,
        id: String,
        title: Option<String>,
    ) -> anyhow::Result<()> {
        // The read horizon before this open moves it: the "New messages" divider goes there.
        let marker = self
            .store
            .lock()
            .unwrap()
            .conversation(&id)?
            .filter(|c| c.unread)
            .map(|c| c.read_horizon_ms);
        let open_at = self.cfg.lock().unwrap().open_chat_at;
        self.flush_draft();
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
            v.selected.clear();
            v.forward = None;
            v.keep_unread = None;
            v.unread_marker = marker;
            v.open_at = open_at;
            v.opening = true;
            v.loaded = false;
            v.at_bottom = true;
            v.new_count = 0;
        }
        self.ui.selection(false, 0);
        self.ui.new_count(0);
        // Shows the chat's stored draft and reply target (or clears the reply bar).
        self.restore_draft(&id);
        self.push_attachments();
        self.on_chat_opened(&id);
        self.find_chat_opened(&id);
        self.scheduled_chat_opened();
        self.mark(|d| {
            d.header = true;
            d.messages = true;
            d.scroll = true;
            d.chats = true;
        });
        let page = match self.client.messages(&id, None, PAGE as u32).await {
            Ok(p) => p,
            Err(e) if is_soft_deleted(&e.to_string()) => {
                self.store.lock().unwrap().delete_conversation(&id)?;
                self.mark(|d| {
                    d.chats = true;
                    d.teams = true;
                });
                anyhow::bail!("This conversation was deleted.");
            }
            Err(e) => return Err(e.into()),
        };
        self.store_messages(&page.messages, None);
        if let Some(last) = page.messages.iter().rev().find(|m| m.is_chat_content()) {
            self.store.lock().unwrap().mark_read(&id, last.compose_ms)?;
            let _ = self.client.mark_read(&id, last).await;
        }
        self.view.lock().unwrap().loaded = true;
        self.mark(|d| {
            d.messages = true;
            d.scroll = true;
            d.chats = true;
        });
        self.prefetch_threads(&id);
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
        // The first row shown now stays where it is once the older rows are above it.
        let first = self
            .store
            .lock()
            .unwrap()
            .messages_before(&conv, None, limit)?
            .first()
            .map(|m| m.id.clone());
        {
            let mut v = self.view.lock().unwrap();
            v.limit = limit + PAGE;
            v.older_anchor = first;
        }
        self.mark(|d| d.messages = true);
        Ok(())
    }
}

/// Whether a conversation belongs in the lists: a channel only while its team lists it (once
/// the teams were read).
fn channel_listed(kind: ConvKind, teams_known: bool, in_a_team: bool) -> bool {
    kind != ConvKind::Channel || !teams_known || in_a_team
}

/// The chat service's answer for a deleted channel or chat.
fn is_soft_deleted(err: &str) -> bool {
    err.contains("SoftDeleted") || err.contains("soft-deleted")
}

#[cfg(test)]
mod deleted_tests {
    use super::*;

    #[test]
    fn a_channel_its_team_no_longer_lists_is_dropped() {
        assert!(channel_listed(ConvKind::Channel, true, true));
        assert!(!channel_listed(ConvKind::Channel, true, false));
        // Without the teams (the read failed) nothing is dropped.
        assert!(channel_listed(ConvKind::Channel, false, false));
        assert!(channel_listed(ConvKind::Group, true, false));
    }

    #[test]
    fn the_soft_deleted_answer_is_recognised() {
        let e = r#"messages: HTTP 404: {"errorCode":404,"message":"{\"subCode\":\"SoftDeleted\",\"details\":\"Thread was not found as it is soft-deleted.\"}"}"#;
        assert!(is_soft_deleted(e));
        assert!(!is_soft_deleted("messages: HTTP 404: not found"));
    }
}
