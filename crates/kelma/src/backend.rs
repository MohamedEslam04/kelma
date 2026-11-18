//! The app engine: sign-in, sync, live events, commands from the UI and rendering.
//!
//! The SQLite store is the source of truth. Network results and live events are written to the
//! store, then marked "dirty"; a render loop rebuilds only the visible view data and pushes it to
//! the UI. This keeps memory small: only the current conversation window is materialized.

use crate::data::{
    ChatData, FileData, HeaderData, ImgData, MsgData, Pixels, ReactData, TeamData, color_for,
    initials,
};
use crate::markdown::{reaction_emoji, render_blocks};
use crate::ui::UiHandle;
use chrono::{Datelike, Local, TimeZone};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use store::{ConvFilter, ConvKind, ConversationRow, MessageRow, Store, TeamRow, UserRow};
use teams_proto::model::{ConversationKind, Event, Message};
use teams_proto::trouter::TrouterEvent;
use teams_proto::{Client, time::now_ms};

/// Commands sent by the UI.
#[derive(Debug, Clone)]
pub enum Cmd {
    RetryLogin,
    SelectChat(String),
    SelectChannel(String, String),
    ToggleTeam(usize),
    ToggleFavorite(String),
    SetFilter(String),
    Search(String),
    Send(String, String),
    TypingTick,
    LoadOlder,
    React(String, String),
    Reply(String),
    CancelReply,
    Edit(String),
    Delete(String),
    OpenUrl(String),
    OpenThread(String),
    CloseThread,
    AttachImage,
    AttachFile,
}

#[derive(Default)]
struct View {
    current: Option<String>,
    current_title: String,
    thread_root: Option<String>,
    filter: String,
    search: String,
    limit: usize,
    reply_to: Option<MessageRow>,
    editing: Option<String>,
    pending: Vec<(String, MessageRow)>,
    typing: HashMap<String, (String, Instant)>,
    last_typing_sent: Option<Instant>,
    collapsed_teams: HashSet<String>,
    teams_order: Vec<String>,
}

#[derive(Default)]
struct Dirty {
    chats: bool,
    messages: bool,
    teams: bool,
    header: bool,
    scroll: bool,
}

/// Shared engine state.
pub struct Engine {
    client: Client,
    store: Mutex<Store>,
    cfg: Mutex<session::Config>,
    ui: UiHandle,
    me: String,
    my_name: String,
    view: Mutex<View>,
    dirty: Mutex<Dirty>,
    avatars: Mutex<HashMap<String, Option<Pixels>>>,
    images: Mutex<HashMap<String, Option<Pixels>>>,
    pub focused: Arc<AtomicBool>,
    /// Demo mode: no network, so network errors are not shown.
    pub offline: AtomicBool,
}

const PAGE: usize = 60;

fn kind_from(k: ConversationKind) -> Option<ConvKind> {
    Some(match k {
        ConversationKind::OneOnOne => ConvKind::OneOnOne,
        ConversationKind::Group => ConvKind::Group,
        ConversationKind::Channel => ConvKind::Channel,
        ConversationKind::Meeting => ConvKind::Meeting,
        ConversationKind::Notes => ConvKind::Notes,
        ConversationKind::Other => return None,
    })
}

/// Serializes the per-message extras we keep (reactions, files, subject) into the store column.
fn props_json(m: &Message) -> String {
    json!({
        "r": m.reactions.iter().map(|r| json!({"key": r.key, "users": r.users})).collect::<Vec<_>>(),
        "f": m.files.iter().map(|f| json!({"name": f.name, "url": f.url, "type": f.file_type})).collect::<Vec<_>>(),
        "s": m.subject,
    })
    .to_string()
}

fn row_from_message(m: &Message, parent: Option<&str>) -> MessageRow {
    MessageRow {
        id: m.id.clone(),
        conversation_id: m.conversation_id.clone(),
        parent_id: parent.map(str::to_owned).or_else(|| m.parent_id.clone()),
        sender_mri: m.sender_mri.clone(),
        sender_name: m.sender_name.clone(),
        compose_ms: m.compose_ms,
        edited_ms: m.edited_ms,
        deleted: m.deleted,
        message_type: m.message_type.clone(),
        content_html: m.content.clone(),
        client_message_id: m.client_message_id.clone(),
        reactions_json: props_json(m),
    }
}

/// Preview text for the chat list.
/// Whether an incoming message pops a desktop notification: new chat content from someone else
/// that the user is not already looking at.
fn should_notify(
    existed: bool,
    mine: bool,
    chat_content: bool,
    is_current: bool,
    focused: bool,
) -> bool {
    !existed && !mine && chat_content && !(is_current && focused)
}

fn preview_of(m: &Message, me: &str) -> String {
    let text = if m.deleted {
        "This message has been deleted.".to_string()
    } else {
        let p = render::plain_text(&render::parse_teams_html(&m.content)).replace('\n', " ");
        if p.trim().is_empty() && !m.files.is_empty() {
            format!("📎 {}", m.files[0].name)
        } else {
            p
        }
    };
    if m.sender_mri == me {
        format!("You: {text}")
    } else {
        text
    }
}

fn chat_time(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    match days {
        0 => t.format("%H:%M").to_string(),
        1 => "Yesterday".into(),
        2..=6 => t.format("%a").to_string(),
        _ if t.year() == now.year() => t.format("%-m/%-d").to_string(),
        _ => t.format("%-m/%-d/%y").to_string(),
    }
}

fn day_label(ms: i64) -> String {
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let days = Local::now()
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => t.format("%A, %B %-d, %Y").to_string(),
    }
}

fn same_day(a: i64, b: i64) -> bool {
    match (
        Local.timestamp_millis_opt(a).single(),
        Local.timestamp_millis_opt(b).single(),
    ) {
        (Some(x), Some(y)) => x.date_naive() == y.date_naive(),
        _ => false,
    }
}

/// The two member GUIDs of a 1:1 chat id (`19:<a>_<b>@unq.gbl.spaces`).
fn one_on_one_members(id: &str) -> Vec<String> {
    id.strip_prefix("19:")
        .and_then(|s| s.strip_suffix("@unq.gbl.spaces"))
        .map(|s| s.split('_').map(|g| format!("8:orgid:{g}")).collect())
        .unwrap_or_default()
}

fn decode_image(bytes: &[u8], max_w: u32, max_h: u32) -> Option<Pixels> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > max_w || img.height() > max_h {
        img.thumbnail(max_w, max_h)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(Pixels::clone_from_slice(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
    ))
}

fn system_text(m: &MessageRow) -> Option<String> {
    let t = m.message_type.as_str();
    if t.starts_with("ThreadActivity/") {
        let what = t.trim_start_matches("ThreadActivity/");
        Some(match what {
            "AddMember" | "MemberJoined" => "A member was added to the chat.".into(),
            "DeleteMember" | "MemberLeft" => "A member left the chat.".into(),
            "TopicUpdate" => "The chat name was changed.".into(),
            _ => format!("{what}."),
        })
    } else if t.starts_with("Event/Call") {
        Some("Call".into())
    } else if t.starts_with("RichText/Media_Call") {
        Some("Call recording".into())
    } else {
        None
    }
}

impl Engine {
    pub fn new(
        client: Client,
        store: Store,
        cfg: session::Config,
        ui: UiHandle,
        me: String,
        my_name: String,
    ) -> Arc<Engine> {
        Arc::new(Engine {
            client,
            store: Mutex::new(store),
            cfg: Mutex::new(cfg),
            ui,
            me,
            my_name,
            view: Mutex::new(View {
                filter: "all".into(),
                limit: PAGE,
                ..Default::default()
            }),
            dirty: Mutex::new(Dirty {
                chats: true,
                teams: true,
                ..Default::default()
            }),
            avatars: Mutex::new(HashMap::new()),
            images: Mutex::new(HashMap::new()),
            focused: Arc::new(AtomicBool::new(true)),
            offline: AtomicBool::new(false),
        })
    }

    fn mark(&self, f: impl FnOnce(&mut Dirty)) {
        f(&mut self.dirty.lock().unwrap());
    }

    /// Starts only the render loop (demo mode: no network).
    pub fn start_offline(self: &Arc<Self>) {
        self.offline.store(true, Ordering::Relaxed);
        let e = self.clone();
        tokio::spawn(async move { e.render_loop().await });
    }

    /// Starts background loops: render, live events, periodic token persistence.
    pub fn start(self: &Arc<Self>) {
        let e = self.clone();
        tokio::spawn(async move { e.render_loop().await });
        let e = self.clone();
        tokio::spawn(async move { e.live_loop().await });
        let e = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(600)).await;
                if e.client.ensure_fresh().await.is_ok() {
                    let cfg = e.cfg.lock().unwrap().clone();
                    let _ = session::persist(&cfg, &e.client).await;
                }
                e.mark(|d| d.chats = true);
            }
        });
        let e = self.clone();
        tokio::spawn(async move {
            if let Err(err) = e.sync().await {
                tracing::error!(%err, "initial sync failed");
                e.ui.banner(format!("Sync failed: {err}"));
            }
        });
    }

    // ---------------------------------------------------------------- sync

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

    async fn fetch_latest(self: &Arc<Self>, conv: &str) {
        match self.client.messages(conv, None, PAGE as u32).await {
            Ok(page) => {
                self.store_messages(&page.messages, None);
                self.mark(|d| d.messages = true);
            }
            Err(err) => tracing::warn!(%err, "history fetch failed"),
        }
    }

    fn store_messages(&self, msgs: &[Message], parent: Option<&str>) {
        let st = self.store.lock().unwrap();
        for m in msgs {
            if m.message_type.starts_with("Control/") {
                continue;
            }
            let _ = st.upsert_message(&row_from_message(m, parent));
        }
    }

    // ---------------------------------------------------------------- live events

    async fn live_loop(self: Arc<Self>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(teams_proto::trouter::run(self.client.clone(), tx));
        let mut was_disconnected = false;
        while let Some(ev) = rx.recv().await {
            match ev {
                TrouterEvent::Connected => {
                    self.ui.connected(true);
                    if was_disconnected {
                        // Catch up on anything missed while offline.
                        let e = self.clone();
                        tokio::spawn(async move {
                            let _ = e.sync().await;
                        });
                    }
                    was_disconnected = false;
                }
                TrouterEvent::Resync => {
                    let e = self.clone();
                    tokio::spawn(async move {
                        let _ = e.sync().await;
                    });
                }
                TrouterEvent::Disconnected(_) => {
                    was_disconnected = true;
                    self.ui.connected(false);
                }
                TrouterEvent::Event(ev) => self.on_event(*ev),
            }
        }
    }

    fn on_event(self: &Arc<Self>, ev: Event) {
        match ev {
            Event::NewMessage(m) | Event::MessageUpdate(m) => self.on_message(m),
            Event::Typing {
                conversation_id,
                from,
            } => {
                if from != self.me {
                    let name = self
                        .store
                        .lock()
                        .unwrap()
                        .user(&from)
                        .ok()
                        .flatten()
                        .map(|u| u.display_name)
                        .unwrap_or_else(|| "Someone".into());
                    self.view.lock().unwrap().typing.insert(
                        conversation_id,
                        (name, Instant::now() + Duration::from_secs(6)),
                    );
                }
            }
            Event::ConversationUpdate { conversation_id }
            | Event::ThreadUpdate { conversation_id } => {
                let known = self
                    .store
                    .lock()
                    .unwrap()
                    .conversation(&conversation_id)
                    .ok()
                    .flatten()
                    .is_some();
                if !known {
                    let e = self.clone();
                    tokio::spawn(async move {
                        let _ = e.sync().await;
                    });
                }
            }
            Event::Presence { .. } => {}
        }
    }

    fn on_message(self: &Arc<Self>, m: Message) {
        if m.message_type.starts_with("Control/") {
            return;
        }
        let mine = m.sender_mri == self.me;
        let (is_current, focused) = {
            let mut v = self.view.lock().unwrap();
            // Our own echo replaces the optimistic copy.
            if let Some(cmid) = &m.client_message_id {
                v.pending.retain(|(c, _)| c != cmid);
            }
            v.typing.remove(&m.conversation_id);
            (
                v.current.as_deref() == Some(m.conversation_id.as_str()),
                self.focused.load(Ordering::Relaxed),
            )
        };
        let known = {
            let st = self.store.lock().unwrap();
            let existed = st.message(&m.id).ok().flatten().is_some();
            let _ = st.upsert_message(&row_from_message(&m, None));
            let is_new_top = !existed && m.parent_id.is_none() && m.is_chat_content();
            if is_new_top || (!existed && m.parent_id.is_some()) {
                let unread = !mine && !(is_current && focused);
                let _ = st.touch_conversation(
                    &m.conversation_id,
                    m.compose_ms,
                    &preview_of(&m, &self.me),
                    unread,
                );
            }
            if should_notify(existed, mine, m.is_chat_content(), is_current, focused) {
                self.notify(&st, &m);
            }
            st.conversation(&m.conversation_id).ok().flatten().is_some()
        };
        if is_current && focused && !mine {
            let e = self.clone();
            let msg = m.clone();
            tokio::spawn(async move {
                let _ = e.client.mark_read(&msg.conversation_id, &msg).await;
            });
        }
        if !known {
            let e = self.clone();
            tokio::spawn(async move {
                let _ = e.sync().await;
            });
        }
        self.mark(|d| {
            d.chats = true;
            if is_current {
                d.messages = true;
                d.scroll = true;
            }
        });
    }

    fn notify(&self, st: &Store, m: &Message) {
        let title = st
            .conversation(&m.conversation_id)
            .ok()
            .flatten()
            .map(|c| c.title)
            .unwrap_or_default();
        let summary = if title.is_empty() || title == m.sender_name {
            m.sender_name.clone()
        } else {
            format!("{} — {}", m.sender_name, title)
        };
        let body: String = render::plain_text(&render::parse_teams_html(&m.content))
            .chars()
            .take(200)
            .collect();
        tracing::info!(conversation = %m.conversation_id, "desktop notification");
        tokio::task::spawn_blocking(move || {
            let _ = notify_rust::Notification::new()
                .appname("kelma")
                .summary(&summary)
                .body(&body)
                .icon("mail-message-new")
                .show();
        });
    }

    // ---------------------------------------------------------------- commands

    pub async fn handle(self: Arc<Self>, cmd: Cmd) {
        if let Err(err) = self.clone().handle_inner(cmd).await {
            tracing::warn!(%err, "command failed");
            if !self.offline.load(Ordering::Relaxed) {
                self.ui.banner(format!("{err}"));
            }
        }
    }

    async fn handle_inner(self: Arc<Self>, cmd: Cmd) -> anyhow::Result<()> {
        match cmd {
            Cmd::RetryLogin => {}
            Cmd::SelectChat(id) => self.open(id, None).await?,
            Cmd::SelectChannel(id, title) => self.open(id, Some(title)).await?,
            Cmd::ToggleTeam(i) => {
                let mut v = self.view.lock().unwrap();
                if let Some(id) = v.teams_order.get(i).cloned()
                    && !v.collapsed_teams.remove(&id)
                {
                    v.collapsed_teams.insert(id);
                }
                drop(v);
                self.mark(|d| d.teams = true);
            }
            Cmd::ToggleFavorite(id) => {
                let st = self.store.lock().unwrap();
                let fav = st.conversation(&id)?.map(|c| c.favorite).unwrap_or(false);
                st.set_favorite(&id, !fav)?;
                drop(st);
                self.mark(|d| d.chats = true);
            }
            Cmd::SetFilter(f) => {
                self.view.lock().unwrap().filter = f;
                self.mark(|d| d.chats = true);
            }
            Cmd::Search(q) => {
                self.view.lock().unwrap().search = q;
                self.mark(|d| d.chats = true);
            }
            Cmd::Send(text, subject) => self.send(text, subject).await?,
            Cmd::TypingTick => {
                let target = {
                    let mut v = self.view.lock().unwrap();
                    let due = v
                        .last_typing_sent
                        .is_none_or(|t| t.elapsed() > Duration::from_secs(15));
                    let target = v.current.clone().filter(|_| due);
                    if target.is_some() {
                        v.last_typing_sent = Some(Instant::now());
                    }
                    target
                };
                if let Some(c) = target {
                    let _ = self.client.send_typing(&c).await;
                }
            }
            Cmd::LoadOlder => self.load_older().await?,
            Cmd::React(id, key) => self.react(&id, &key).await?,
            Cmd::Reply(id) => {
                let row = self.store.lock().unwrap().message(&id)?;
                if let Some(r) = row {
                    let preview: String =
                        render::plain_text(&render::parse_teams_html(&r.content_html))
                            .chars()
                            .take(120)
                            .collect();
                    self.ui.reply_bar(r.sender_name.clone(), preview, false);
                    let mut v = self.view.lock().unwrap();
                    v.editing = None;
                    v.reply_to = Some(r);
                }
                self.ui.focus();
            }
            Cmd::Edit(id) => {
                let row = self.store.lock().unwrap().message(&id)?;
                if let Some(r) = row.filter(|r| r.sender_mri == self.me) {
                    let text = render::plain_text(&render::parse_teams_html(&r.content_html));
                    self.ui
                        .reply_bar(String::new(), text.chars().take(120).collect(), true);
                    self.ui.set_draft(text);
                    let mut v = self.view.lock().unwrap();
                    v.reply_to = None;
                    v.editing = Some(r.id);
                }
                self.ui.focus();
            }
            Cmd::CancelReply => {
                let mut v = self.view.lock().unwrap();
                v.reply_to = None;
                if v.editing.take().is_some() {
                    self.ui.set_draft(String::new());
                }
                drop(v);
                self.ui.reply_bar(String::new(), String::new(), false);
            }
            Cmd::Delete(id) => {
                let conv = self.current_target();
                if let Some(conv) = conv {
                    self.client.delete_message(&conv, &id).await?;
                    self.store.lock().unwrap().delete_message_local(&id)?;
                    self.mark(|d| d.messages = true);
                }
            }
            Cmd::OpenUrl(url) => {
                if let Some(media) = url.strip_prefix("kelma-media:") {
                    self.play_media(media).await?;
                } else if url.starts_with("https://")
                    || url.starts_with("http://")
                    || url.starts_with("mailto:")
                {
                    let _ = open::that_detached(&url);
                }
            }
            Cmd::OpenThread(root) => {
                {
                    let mut v = self.view.lock().unwrap();
                    v.thread_root = Some(root.clone());
                    v.limit = PAGE;
                }
                self.mark(|d| {
                    d.header = true;
                    d.messages = true;
                    d.scroll = true;
                });
                let conv = self.view.lock().unwrap().current.clone();
                if let Some(c) = conv {
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
                        Some(&root),
                    );
                    self.mark(|d| {
                        d.messages = true;
                        d.scroll = true;
                    });
                }
            }
            Cmd::CloseThread => {
                self.view.lock().unwrap().thread_root = None;
                self.mark(|d| {
                    d.header = true;
                    d.messages = true;
                    d.scroll = true;
                });
            }
            Cmd::AttachImage => self.attach_image().await?,
            Cmd::AttachFile => {
                self.ui.banner(
                    "Sending files is not supported yet: share a OneDrive link instead.".into(),
                );
            }
        }
        Ok(())
    }

    /// Conversation id messages are sent to (channel thread aware).
    fn current_target(&self) -> Option<String> {
        let v = self.view.lock().unwrap();
        let c = v.current.clone()?;
        Some(match &v.thread_root {
            Some(r) => format!("{c};messageid={r}"),
            None => c,
        })
    }

    async fn open(self: &Arc<Self>, id: String, title: Option<String>) -> anyhow::Result<()> {
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

    async fn load_older(self: &Arc<Self>) -> anyhow::Result<()> {
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

    async fn send(self: &Arc<Self>, text: String, subject: String) -> anyhow::Result<()> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Ok(());
        }
        let (target, reply, editing, current) = {
            let mut v = self.view.lock().unwrap();
            (
                self.current_target_locked(&v),
                v.reply_to.take(),
                v.editing.take(),
                v.current.clone(),
            )
        };
        self.ui.reply_bar(String::new(), String::new(), false);
        let (Some(target), Some(current)) = (target, current) else {
            return Ok(());
        };
        if let Some(id) = editing {
            let html = render::text_to_teams_html(&text);
            self.client
                .edit_message(&target, &id, &html, &self.my_name)
                .await?;
            let st = self.store.lock().unwrap();
            if let Some(mut r) = st.message(&id)? {
                r.content_html = html;
                r.edited_ms = Some(now_ms());
                st.upsert_message(&r)?;
            }
            drop(st);
            self.mark(|d| d.messages = true);
            return Ok(());
        }
        let html = match &reply {
            Some(r) => {
                let preview: String =
                    render::plain_text(&render::parse_teams_html(&r.content_html))
                        .chars()
                        .take(200)
                        .collect();
                render::reply_html(
                    &r.id,
                    &r.sender_mri,
                    &r.sender_name,
                    r.compose_ms,
                    &preview,
                    &text,
                )
            }
            None => render::text_to_teams_html(&text),
        };
        let pending_id = format!("pending-{}", now_ms());
        let thread_root = self.view.lock().unwrap().thread_root.clone();
        let row = MessageRow {
            id: pending_id.clone(),
            conversation_id: current.clone(),
            parent_id: thread_root,
            sender_mri: self.me.clone(),
            sender_name: self.my_name.clone(),
            compose_ms: now_ms(),
            edited_ms: None,
            deleted: false,
            message_type: "RichText/Html".into(),
            content_html: html.clone(),
            client_message_id: None,
            reactions_json: json!({"s": if subject.is_empty() { Value::Null } else { Value::String(subject.clone()) }}).to_string(),
        };
        self.view
            .lock()
            .unwrap()
            .pending
            .push((pending_id.clone(), row));
        self.mark(|d| {
            d.messages = true;
            d.scroll = true;
        });
        let subj = (!subject.is_empty()).then_some(subject.as_str());
        match self
            .client
            .send_message(&target, &html, &self.my_name, subj)
            .await
        {
            Ok(cmid) => {
                let mut v = self.view.lock().unwrap();
                if let Some(p) = v.pending.iter_mut().find(|(id, _)| id == &pending_id) {
                    p.0 = cmid;
                }
                drop(v);
                self.ui.clear_subject();
                let st = self.store.lock().unwrap();
                let _ = st.touch_conversation(&current, now_ms(), &format!("You: {text}"), false);
                drop(st);
                self.mark(|d| d.chats = true);
                // Fallback in case the live echo never arrives.
                let e = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(8)).await;
                    let stale = e.view.lock().unwrap().pending.iter().any(|(c, r)| {
                        r.conversation_id == current && c.chars().all(|ch| ch.is_ascii_digit())
                    });
                    if stale {
                        e.fetch_latest(&target).await;
                        e.view.lock().unwrap().pending.retain(|(c, r)| {
                            !(r.conversation_id == current
                                && c.chars().all(|ch| ch.is_ascii_digit()))
                        });
                        e.mark(|d| d.messages = true);
                    }
                });
            }
            Err(err) => {
                self.view
                    .lock()
                    .unwrap()
                    .pending
                    .retain(|(id, _)| id != &pending_id);
                self.ui.set_draft(text);
                self.mark(|d| d.messages = true);
                return Err(err.into());
            }
        }
        Ok(())
    }

    fn current_target_locked(&self, v: &View) -> Option<String> {
        let c = v.current.clone()?;
        Some(match &v.thread_root {
            Some(r) => format!("{c};messageid={r}"),
            None => c,
        })
    }

    async fn react(self: &Arc<Self>, msg_id: &str, key: &str) -> anyhow::Result<()> {
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        let row = self.store.lock().unwrap().message(msg_id)?;
        let Some(mut row) = row else { return Ok(()) };
        let mut props: Value =
            serde_json::from_str(&row.reactions_json).unwrap_or_else(|_| json!({}));
        let mut reactions: Vec<Value> = props
            .get("r")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mine = reactions.iter().any(|r| {
            r["key"] == key
                && r["users"]
                    .as_array()
                    .is_some_and(|u| u.iter().any(|x| x == self.me.as_str()))
        });
        let conv_for_msg = if row.parent_id.is_some() {
            target.clone()
        } else {
            row.conversation_id.clone()
        };
        if mine {
            self.client
                .remove_reaction(&conv_for_msg, msg_id, key)
                .await?;
            for r in reactions.iter_mut().filter(|r| r["key"] == key) {
                if let Some(u) = r["users"].as_array_mut() {
                    u.retain(|x| x != self.me.as_str());
                }
            }
            reactions.retain(|r| r["users"].as_array().is_some_and(|u| !u.is_empty()));
        } else {
            self.client.add_reaction(&conv_for_msg, msg_id, key).await?;
            match reactions.iter_mut().find(|r| r["key"] == key) {
                Some(r) => {
                    if let Some(u) = r["users"].as_array_mut() {
                        u.push(json!(self.me));
                    }
                }
                None => reactions.push(json!({"key": key, "users": [self.me]})),
            }
        }
        props["r"] = json!(reactions);
        row.reactions_json = props.to_string();
        self.store.lock().unwrap().upsert_message(&row)?;
        self.mark(|d| d.messages = true);
        Ok(())
    }

    /// Downloads a voice message / video with credentials and opens it in the system player.
    async fn play_media(&self, url: &str) -> anyhow::Result<()> {
        let dir = session::Paths::new()?.cache_dir.join("media");
        std::fs::create_dir_all(&dir)?;
        let name: String = url
            .split("/objects/")
            .nth(1)
            .unwrap_or("media")
            .split('/')
            .next()
            .unwrap_or("media")
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let path = dir.join(format!("{name}.m4a"));
        if !path.exists() {
            self.ui.banner("Downloading…".into());
            let data = self.client.download_media(url, 100 * 1024 * 1024).await?;
            std::fs::write(&path, data)?;
            self.ui.banner(String::new());
        }
        open::that_detached(&path)?;
        Ok(())
    }

    async fn attach_image(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file()
            .await
        else {
            return Ok(());
        };
        let data = file.read().await;
        let (w, h) = image::load_from_memory(&data)
            .map(|i| (i.width(), i.height()))
            .unwrap_or((400, 300));
        let (w, h) = if w > 400 {
            (400, h * 400 / w.max(1))
        } else {
            (w, h)
        };
        let conv_only = target
            .split(";messageid=")
            .next()
            .unwrap_or(&target)
            .to_string();
        let id = self
            .client
            .upload_image(&conv_only, &file.file_name(), data)
            .await?;
        let html = teams_proto::ams::image_html(&id, w, h);
        self.client
            .send_message(&target, &html, &self.my_name, None)
            .await?;
        self.ui.banner(String::new());
        Ok(())
    }

    // ---------------------------------------------------------------- rendering

    async fn render_loop(self: Arc<Self>) {
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

#[cfg(test)]
mod tests {
    use super::should_notify;

    #[test]
    fn notifies_only_for_new_messages_from_others_out_of_view() {
        // New message from someone else in another chat, or with the window unfocused.
        assert!(should_notify(false, false, true, false, true));
        assert!(should_notify(false, false, true, true, false));
        // Already looking at it.
        assert!(!should_notify(false, false, true, true, true));
        // Own messages, repeats (edits, resync) and system lines never notify.
        assert!(!should_notify(false, true, true, false, false));
        assert!(!should_notify(true, false, true, false, false));
        assert!(!should_notify(false, false, false, false, false));
    }
}
