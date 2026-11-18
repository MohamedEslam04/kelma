//! The app engine: sign-in, sync, live events, commands from the UI and rendering.
//!
//! The SQLite store is the source of truth. Network results and live events are written to the
//! store, then marked "dirty"; a render loop rebuilds only the visible view data and pushes it to
//! the UI. This keeps memory small: only the current conversation window is materialized.

use crate::data::Pixels;
use crate::ui::UiHandle;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use store::{MessageRow, Store};
use teams_proto::Client;

mod actions;
mod live;
mod render;
mod send;
mod sync;
mod util;

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
    CopyLink(String),
    ToggleSave(String),
    Pin(String),
    MarkUnread(String),
    /// Forward dialog opened for one message, or for the multi-select ticks when `None`.
    ForwardBegin(Option<String>),
    ForwardSearch(String),
    ForwardToggle(String),
    ForwardSend(String),
    ForwardCancel,
    SelectBegin(String),
    ToggleSelect(String),
    SelectCancel,
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
    /// Multi-select ticks, in the order they were made (at most `actions::MAX_FORWARD`).
    selected: Vec<String>,
    /// Open forward dialog: what is forwarded, the last search and the picked chats.
    forward: Option<ForwardState>,
    /// Chat marked unread while open; the list shows it until another chat is opened.
    keep_unread: Option<String>,
}

#[derive(Default)]
struct ForwardState {
    ids: Vec<String>,
    query: String,
    picked: Vec<String>,
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
    /// Reactor MRIs whose profile fetch was already started (tooltip names).
    name_requests: Mutex<HashSet<String>>,
    pub focused: Arc<AtomicBool>,
    /// Demo mode: no network, so network errors are not shown.
    pub offline: AtomicBool,
}

const PAGE: usize = 60;

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
            name_requests: Mutex::new(HashSet::new()),
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
                // Shorter than the 300 s refresh margin, so tokens never lapse while idle.
                tokio::time::sleep(Duration::from_secs(240)).await;
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
                        ::render::plain_text(&::render::parse_teams_html(&r.content_html))
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
                    let text = ::render::plain_text(&::render::parse_teams_html(&r.content_html));
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
            Cmd::AttachFile => self.attach_file().await?,
            Cmd::CopyLink(id) => self.copy_link(&id)?,
            Cmd::ToggleSave(id) => self.toggle_save(&id).await?,
            Cmd::Pin(id) => self.pin(&id).await?,
            Cmd::MarkUnread(id) => self.mark_unread(&id).await?,
            Cmd::ForwardBegin(id) => self.forward_begin(id)?,
            Cmd::ForwardSearch(q) => self.forward_search(q)?,
            Cmd::ForwardToggle(id) => self.forward_toggle(&id)?,
            Cmd::ForwardSend(note) => self.forward_send(&note).await?,
            Cmd::ForwardCancel => self.view.lock().unwrap().forward = None,
            Cmd::SelectBegin(id) => self.select_begin(&id),
            Cmd::ToggleSelect(id) => self.toggle_select(&id),
            Cmd::SelectCancel => self.select_cancel(),
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

    fn current_target_locked(&self, v: &View) -> Option<String> {
        let c = v.current.clone()?;
        Some(match &v.thread_root {
            Some(r) => format!("{c};messageid={r}"),
            None => c,
        })
    }
}
