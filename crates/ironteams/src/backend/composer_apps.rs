//! Apps in the composer (plan S3.6): the "+" button opens a picker of the open chat's installed
//! apps that add commands to the compose box.
//!
//! - A search command (a messaging extension's query) takes text, runs `composeExtension/query`
//!   after a pause in typing and lists the result cards as previews. Choosing one puts the card
//!   in the draft as a card attachment. Read only: nothing is sent to the chat.
//! - An action command opens its task module (`taskInfo.url`, or the page `fetchTask` answers
//!   with) in the web host, with the app's shim (`appstore::webhost_open::plan_task`). What the
//!   page submits (`tasks.submitTask`, `dialog.url.submit`) goes to the bot as
//!   `composeExtension/submitAction`, and the card the bot answers with is added to the draft
//!   like a picked search result. Nothing opens in the browser, nor for an unknown app or a
//!   link that is not https.
//!
//! The draft's existing attachments are images and files that go through the outbox. Cards are
//! kept here, by chat, until Send: `Engine::send` takes them (`take_composer_cards`) and queues
//! one `OutboxKind::Card` message holding the typed text and every card (see
//! `teams_proto::card_send`). In demo mode and in the tests the apps and results come from
//! `fixtures`.

mod card_preview;
mod fixtures;

use super::Engine;
use crate::data::Pixels;
use app_store::bots::{
    self, CommandContext, CommandKind, Extension, ExtensionCommand, Place, PlaceApp, QueryOutcome,
    ResultItem, ResultKind, TaskOutcome,
};
pub use card_preview::{Preview, preview_of};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use store::ConvKind;

/// How long typing must pause before the extension is asked.
const DEBOUNCE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(350)
};
/// Cards one draft can hold.
const MAX_CARDS: usize = 10;
/// Thumbnails are decoded to at most this many pixels.
const THUMB: u32 = 96;

/// What the picker's buttons and the search box send, as `(name, argument)` strings from the
/// `ComposerApps` global.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposerAppsCmd {
    /// The "+" button: opens the picker and lists the commands of the open chat's apps.
    List,
    Close,
    /// Back from a search command to the list.
    Back,
    /// Opens a search command (a command key).
    Search(String),
    /// Runs an action command (a command key).
    Action(String),
    /// The search text changed.
    Query(String),
    /// Puts result `n` in the draft.
    Choose(usize),
    /// Takes card `n` out of the draft.
    Remove(usize),
}

impl ComposerAppsCmd {
    pub fn parse(name: &str, arg: &str) -> Option<Self> {
        Some(match name {
            "list" => Self::List,
            "close" => Self::Close,
            "back" => Self::Back,
            "search" => Self::Search(arg.to_owned()),
            "action" => Self::Action(arg.to_owned()),
            "query" => Self::Query(arg.to_owned()),
            "choose" => Self::Choose(arg.parse().ok()?),
            "remove" => Self::Remove(arg.parse().ok()?),
            _ => return None,
        })
    }
}

/// `(app, extension, command)` positions in the listed apps.
type Key = (usize, usize, usize);

fn key_text(k: Key) -> String {
    format!("{}.{}.{}", k.0, k.1, k.2)
}

fn parse_key(s: &str) -> Option<Key> {
    let mut p = s.split('.').map(|n| n.parse::<usize>().ok());
    let key = (p.next()??, p.next()??, p.next()??);
    p.next().is_none().then_some(key)
}

impl CardAttachment {
    /// The card as the send path carries it.
    pub(crate) fn out_card(&self) -> teams_proto::card_send::OutCard {
        teams_proto::card_send::OutCard {
            app_id: self.app_id.clone(),
            app_name: self.app_name.clone(),
            content_type: self.content_type.clone(),
            content: self.content.clone(),
        }
    }
}

/// A card waiting in a draft.
#[derive(Debug, Clone)]
pub(crate) struct CardAttachment {
    pub app_id: String,
    pub app_name: String,
    pub content_type: String,
    pub content: Value,
    pub preview: Preview,
}

#[derive(Default)]
pub(super) struct State {
    open: bool,
    /// The chat the picker was opened for.
    conv: String,
    apps: Vec<PlaceApp>,
    loading: bool,
    status: String,
    /// The search command that is open.
    active: Option<Key>,
    /// Bumped by every search, so an older answer or a stale pause is dropped.
    generation: u64,
    searching: bool,
    results: Vec<ResultItem>,
    /// Thumbnails by address; `None` while loading or when the download failed.
    thumbs: HashMap<String, Option<Pixels>>,
    note: String,
    /// The cards of each chat's draft.
    cards: HashMap<String, Vec<CardAttachment>>,
}

impl State {
    fn command(&self, k: Key) -> Option<(&PlaceApp, &Extension, &ExtensionCommand)> {
        let app = self.apps.get(k.0)?;
        let ext = app.extensions.get(k.1)?;
        Some((app, ext, ext.commands.get(k.2)?))
    }
}

/// One line of the command list: an app's name, or one of its commands.
#[derive(Clone, Default)]
pub struct CmdRow {
    pub key: String,
    pub app: String,
    pub title: String,
    pub description: String,
    pub action: bool,
    pub header: bool,
}

#[derive(Clone)]
pub struct ResultRow {
    pub title: String,
    pub subtitle: String,
    pub thumb: Option<Pixels>,
    pub letter: String,
}

#[derive(Clone)]
pub struct CardRow {
    pub app: String,
    pub title: String,
    pub subtitle: String,
}

/// Everything the picker shows.
#[derive(Clone, Default)]
pub struct PickerData {
    pub open: bool,
    pub commands: Vec<CmdRow>,
    /// The open search command's key; empty while the list shows.
    pub command_key: String,
    pub command_title: String,
    pub app_name: String,
    pub placeholder: String,
    pub loading: bool,
    pub searching: bool,
    pub status: String,
    pub results: Vec<ResultRow>,
    pub cards: Vec<CardRow>,
    pub note: String,
}

/// The `commandContext`s a command of the compose box can run in.
const CONTEXT: CommandContext = CommandContext::Compose;

/// The scopes of a messaging extension that work in a kind of conversation.
fn scopes_for(kind: ConvKind) -> &'static [&'static str] {
    match kind {
        ConvKind::OneOnOne | ConvKind::Notes => &["personal"],
        ConvKind::Group | ConvKind::Meeting => &["groupchat"],
        ConvKind::Channel => &["team"],
    }
}

/// Keeps the apps that add a command to the compose box in one of the scopes (an extension
/// that names no scope works everywhere). Bots alone add nothing here.
fn composer_apps(apps: Vec<PlaceApp>, scopes: &[&str]) -> Vec<PlaceApp> {
    apps.into_iter()
        .filter_map(|mut app| {
            app.bots.clear();
            app.extensions.retain_mut(|e| {
                if !e.scopes.is_empty() && !scopes.iter().any(|s| e.in_scope(s)) {
                    return false;
                }
                e.commands.retain(|c| c.contexts.contains(&CONTEXT));
                !e.commands.is_empty()
            });
            (!app.extensions.is_empty()).then_some(app)
        })
        .collect()
}

fn command_rows(apps: &[PlaceApp]) -> Vec<CmdRow> {
    let mut rows = Vec::new();
    for (a, app) in apps.iter().enumerate() {
        rows.push(CmdRow {
            app: app.name.clone(),
            header: true,
            ..Default::default()
        });
        for (e, ext) in app.extensions.iter().enumerate() {
            for (c, cmd) in ext.commands.iter().enumerate() {
                rows.push(CmdRow {
                    key: key_text((a, e, c)),
                    app: app.name.clone(),
                    title: cmd.title.clone(),
                    description: cmd.description.clone(),
                    action: cmd.kind == CommandKind::Action,
                    header: false,
                });
            }
        }
    }
    rows
}

/// The parameters of a run: the text goes to the command's first parameter.
fn query_params(cmd: &ExtensionCommand, text: &str) -> Vec<(String, String)> {
    cmd.parameters
        .first()
        .map(|p| vec![(p.name.clone(), text.to_owned())])
        .unwrap_or_default()
}

/// What a query's answer shows: the result cards, and a line of text when there are none or
/// the app wants something IronTeams cannot do yet.
fn outcome_view(outcome: QueryOutcome) -> (Vec<ResultItem>, String) {
    let none = |text: Option<String>| text.unwrap_or_else(|| "No results.".to_owned());
    match outcome {
        QueryOutcome::Results(r) => match r.kind {
            ResultKind::Result if !r.items.is_empty() => (r.items, String::new()),
            ResultKind::Result | ResultKind::Message => (Vec::new(), none(r.text)),
            ResultKind::Auth => (
                Vec::new(),
                "This app needs you to sign in first. Signing in is not supported here yet."
                    .to_owned(),
            ),
            ResultKind::SilentAuth => (
                Vec::new(),
                "This app asks for single sign-on, which is not supported here yet.".to_owned(),
            ),
            ResultKind::Config => (
                Vec::new(),
                "This app has to be set up first. Its settings are not available here.".to_owned(),
            ),
            ResultKind::BotMessagePreview | ResultKind::Other => (
                Vec::new(),
                "This app answered in a way that is not supported here yet.".to_owned(),
            ),
        },
        QueryOutcome::Other => (
            Vec::new(),
            "This app answered in a way that is not supported here yet.".to_owned(),
        ),
        QueryOutcome::Empty => (Vec::new(), none(None)),
        QueryOutcome::Failed { status, message } => (
            Vec::new(),
            if message.is_empty() {
                format!("The app answered {status}.")
            } else {
                format!("The app answered {status}: {message}")
            },
        ),
    }
}

/// The conversation id to give a bot: the notes chat has no bots in it, so the service refuses
/// it (seen live), and the web client sends none without a thread.
fn notes_free(conv: &str) -> &str {
    if conv == "48:notes" { "" } else { conv }
}

/// The cards an action command's `submitAction` answer puts in the message, or the line to show
/// when it gives none. A bot may also post on its own and answer nothing.
fn submitted_cards(outcome: QueryOutcome) -> Result<Vec<ResultItem>, String> {
    match outcome {
        QueryOutcome::Results(r)
            if matches!(r.kind, ResultKind::Result | ResultKind::BotMessagePreview)
                && !r.items.is_empty() =>
        {
            Ok(r.items)
        }
        QueryOutcome::Results(r) if r.kind == ResultKind::Message => Err(r
            .text
            .unwrap_or_else(|| "The app sent no card to add.".into())),
        QueryOutcome::Empty => Err("The app sent no card to add.".into()),
        QueryOutcome::Results(r) if r.kind == ResultKind::Result => {
            Err("The app sent no card to add.".into())
        }
        other => Err(outcome_view(other).1),
    }
}

/// Puts a card in a chat's draft; the line to show.
fn add_card(
    cards: &mut HashMap<String, Vec<CardAttachment>>,
    conv: &str,
    app_id: String,
    app_name: String,
    item: ResultItem,
) -> String {
    let preview = preview_of(&item);
    let list = cards.entry(conv.to_owned()).or_default();
    if list.len() >= MAX_CARDS {
        return format!("A message holds up to {MAX_CARDS} cards.");
    }
    let note = format!("Added {} to your message.", preview.title);
    list.push(CardAttachment {
        app_id,
        app_name,
        content_type: item.content_type,
        content: item.content,
        preview,
    });
    note
}

/// The view of the state, and the thumbnail addresses it still needs.
fn picker_data(s: &mut State, offline: bool) -> (PickerData, Vec<String>) {
    let mut wanted = Vec::new();
    let mut results = Vec::new();
    for item in &s.results {
        let p = preview_of(item);
        let thumb = match p.image.as_deref() {
            // Only https images: the download adds credentials for Microsoft hosts.
            Some(url) if url.starts_with("https://") => match s.thumbs.get(url) {
                Some(found) => found.clone(),
                None => {
                    if !offline {
                        wanted.push(url.to_owned());
                    }
                    s.thumbs.insert(url.to_owned(), None);
                    None
                }
            },
            _ => None,
        };
        results.push(ResultRow {
            letter: p.title.chars().next().map(String::from).unwrap_or_default(),
            title: p.title,
            subtitle: p.subtitle,
            thumb,
        });
    }
    let open_command = s.active.and_then(|k| s.command(k));
    let data = PickerData {
        open: s.open,
        commands: command_rows(&s.apps),
        command_key: s.active.map(key_text).unwrap_or_default(),
        command_title: open_command
            .map(|(_, _, c)| c.title.clone())
            .unwrap_or_default(),
        app_name: open_command
            .map(|(a, _, _)| a.name.clone())
            .unwrap_or_default(),
        placeholder: open_command
            .and_then(|(_, _, c)| c.parameters.first())
            .map(|p| {
                if p.title.is_empty() {
                    p.name.clone()
                } else {
                    p.title.clone()
                }
            })
            .unwrap_or_else(|| "Search".to_owned()),
        loading: s.loading,
        searching: s.searching,
        status: s.status.clone(),
        results,
        cards: s
            .cards
            .get(&s.conv)
            .map(|list| {
                list.iter()
                    .map(|c| CardRow {
                        app: c.app_name.clone(),
                        title: c.preview.title.clone(),
                        subtitle: c.preview.subtitle.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        note: s.note.clone(),
    };
    (data, wanted)
}

impl Engine {
    pub(super) async fn composer_apps_cmd(
        self: &Arc<Self>,
        cmd: ComposerAppsCmd,
    ) -> anyhow::Result<()> {
        match cmd {
            ComposerAppsCmd::List => self.ca_list().await,
            ComposerAppsCmd::Close => {
                let mut s = self.composer_apps.lock().unwrap();
                s.open = false;
                s.generation += 1;
                drop(s);
                self.ca_push();
                Ok(())
            }
            ComposerAppsCmd::Back => {
                let mut s = self.composer_apps.lock().unwrap();
                s.active = None;
                s.generation += 1;
                s.searching = false;
                s.results.clear();
                s.status.clear();
                s.note.clear();
                drop(s);
                self.ca_push();
                Ok(())
            }
            ComposerAppsCmd::Search(key) => self.ca_search(&key).await,
            ComposerAppsCmd::Action(key) => {
                self.ca_action(&key).await;
                Ok(())
            }
            ComposerAppsCmd::Query(text) => self.ca_query(text).await,
            ComposerAppsCmd::Choose(n) => {
                self.ca_choose(n);
                Ok(())
            }
            ComposerAppsCmd::Remove(n) => {
                let mut s = self.composer_apps.lock().unwrap();
                let conv = s.conv.clone();
                if let Some(list) = s.cards.get_mut(&conv)
                    && n < list.len()
                {
                    list.remove(n);
                }
                s.note.clear();
                drop(s);
                self.ca_push();
                Ok(())
            }
        }
    }

    /// Shows the picker's state, and starts the thumbnail downloads it asked for.
    fn ca_push(self: &Arc<Self>) {
        let offline = self.offline.load(Ordering::Relaxed);
        let (data, wanted) = picker_data(&mut self.composer_apps.lock().unwrap(), offline);
        self.ui.composer_apps(data);
        for url in wanted {
            let e = self.clone();
            tokio::spawn(async move {
                let found = match e.client.bytes(&url, &[], 2 * 1024 * 1024).await {
                    Ok(b) => tokio::task::spawn_blocking(move || {
                        super::util::decode_image(&b, THUMB, THUMB)
                    })
                    .await
                    .ok()
                    .flatten(),
                    Err(err) => {
                        tracing::debug!(%err, "result thumbnail download failed");
                        None
                    }
                };
                if found.is_some() {
                    e.composer_apps.lock().unwrap().thumbs.insert(url, found);
                    e.ca_push();
                }
            });
        }
    }

    async fn ca_list(self: &Arc<Self>) -> anyhow::Result<()> {
        let Some(conv) = self.view.lock().unwrap().current.clone() else {
            // No chat is open: the picker has nothing to list.
            self.composer_apps.lock().unwrap().open = false;
            self.ca_push();
            return Ok(());
        };
        let kind = self
            .store
            .lock()
            .unwrap()
            .conversation(&conv)?
            .map_or(ConvKind::Group, |c| c.kind);
        {
            let mut s = self.composer_apps.lock().unwrap();
            s.open = true;
            s.conv = conv.clone();
            s.apps.clear();
            s.active = None;
            s.results.clear();
            s.status.clear();
            s.note.clear();
            s.searching = false;
            s.loading = true;
            s.generation += 1;
        }
        self.ca_push();
        let listed = self.ca_source(&conv, kind).await;
        let mut s = self.composer_apps.lock().unwrap();
        if !s.open || s.conv != conv {
            return Ok(());
        }
        s.loading = false;
        match listed {
            Ok(apps) => {
                if apps.is_empty() {
                    s.status = "No app in this chat adds a command to the message box.".into();
                }
                s.apps = apps;
            }
            Err(err) => s.status = format!("Could not list the apps: {err}"),
        }
        drop(s);
        self.ca_push();
        Ok(())
    }

    /// The apps of the chat with a command for the compose box (read only).
    async fn ca_source(&self, conv: &str, kind: ConvKind) -> anyhow::Result<Vec<PlaceApp>> {
        let apps = if self.offline.load(Ordering::Relaxed) {
            bots::parse_place_apps(&fixtures::place_apps())
        } else {
            let place = if kind == ConvKind::Channel {
                Place::Team(conv.to_owned())
            } else {
                Place::Chat(conv.to_owned())
            };
            bots::place_apps(&self.client, &place).await?
        };
        Ok(composer_apps(apps, scopes_for(kind)))
    }

    async fn ca_search(self: &Arc<Self>, key: &str) -> anyhow::Result<()> {
        let Some(k) = parse_key(key) else {
            return Ok(());
        };
        let (generation, run_now) = {
            let mut s = self.composer_apps.lock().unwrap();
            let Some((_, _, cmd)) = s.command(k) else {
                return Ok(());
            };
            if cmd.kind != CommandKind::Query {
                return Ok(());
            }
            // A command without a field, or one that asks to run when it opens, runs at once.
            let run_now = cmd.parameters.is_empty() || cmd.initial_run;
            s.active = Some(k);
            s.results.clear();
            s.status.clear();
            s.note.clear();
            s.generation += 1;
            (s.generation, run_now)
        };
        self.ca_push();
        if run_now {
            self.ca_run("", generation).await;
        }
        Ok(())
    }

    /// The search text changed: asks the app once typing pauses.
    async fn ca_query(self: &Arc<Self>, text: String) -> anyhow::Result<()> {
        let generation = {
            let mut s = self.composer_apps.lock().unwrap();
            if !s.open || s.active.is_none() {
                return Ok(());
            }
            s.generation += 1;
            s.generation
        };
        tokio::time::sleep(DEBOUNCE).await;
        if self.composer_apps.lock().unwrap().generation != generation {
            return Ok(());
        }
        self.ca_run(&text, generation).await;
        Ok(())
    }

    /// Runs the open search command with `text`, unless a newer search started meanwhile.
    async fn ca_run(self: &Arc<Self>, text: &str, generation: u64) {
        let (app, ext, cmd, conv) = {
            let mut s = self.composer_apps.lock().unwrap();
            let Some((app, ext, cmd)) = s.active.and_then(|k| s.command(k)) else {
                return;
            };
            let found = (app.clone(), ext.clone(), cmd.clone(), s.conv.clone());
            s.searching = true;
            s.status.clear();
            found
        };
        self.ca_push();
        let answer = if self.offline.load(Ordering::Relaxed) {
            Ok(fixtures::query(&app.id, &cmd.id, text))
        } else {
            // The notes chat has no bots in it, so the service refuses it (seen live).
            let conversation = if conv == "48:notes" {
                ""
            } else {
                conv.as_str()
            };
            bots::query_extension(
                &self.client,
                &app,
                &ext,
                &cmd,
                CONTEXT,
                &query_params(&cmd, text),
                conversation,
                &self.my_name,
            )
            .await
        };
        let mut s = self.composer_apps.lock().unwrap();
        if s.generation != generation {
            return;
        }
        s.searching = false;
        match answer {
            Ok(outcome) => (s.results, s.status) = outcome_view(outcome),
            Err(err) => {
                s.results.clear();
                s.status = format!("The search failed: {err}");
            }
        }
        drop(s);
        self.ca_push();
    }

    /// Result `n` becomes a card in the draft of the chat.
    fn ca_choose(self: &Arc<Self>, n: usize) {
        let mut s = self.composer_apps.lock().unwrap();
        let Some(item) = s.results.get(n).cloned() else {
            return;
        };
        let Some((app, _, _)) = s.active.and_then(|k| s.command(k)) else {
            return;
        };
        let (app_id, app_name) = (app.id.clone(), app.name.clone());
        let conv = s.conv.clone();
        let note = add_card(&mut s.cards, &conv, app_id, app_name, item);
        s.note = note;
        drop(s);
        self.ca_push();
    }

    /// Opens an action command's task module in the web host. A command with a static
    /// `taskInfo.url` opens it as it is; one with `fetchTask` asks the bot for the page first
    /// (`composeExtension/fetchTask`, read only). What the page submits comes back as a card.
    async fn ca_action(self: &Arc<Self>, key: &str) {
        let Some(k) = parse_key(key) else {
            return;
        };
        let (app, ext, cmd, conv) = {
            let s = self.composer_apps.lock().unwrap();
            let Some((app, ext, cmd)) = s.command(k) else {
                return;
            };
            if cmd.kind != CommandKind::Action {
                return;
            }
            (app.clone(), ext.clone(), cmd.clone(), s.conv.clone())
        };
        let note = self.ca_open_task(&app, &ext, &cmd, &conv).await;
        self.composer_apps.lock().unwrap().note = note;
        self.ca_push();
    }

    /// Opens the task module of `cmd`; the line to show about it.
    async fn ca_open_task(
        self: &Arc<Self>,
        app: &PlaceApp,
        ext: &Extension,
        cmd: &ExtensionCommand,
        conv: &str,
    ) -> String {
        let title = &cmd.title;
        let (url, task_title) = match &cmd.task {
            Some(t) => (t.url.clone(), t.title.clone()),
            None if cmd.fetch_task => match self.ca_fetch_task(app, ext, cmd, conv).await {
                Ok(TaskOutcome::Url { title, url }) => (url, title),
                Ok(TaskOutcome::Card) => {
                    return format!("{title} opens a card form, which is not supported yet.");
                }
                Ok(TaskOutcome::Message(text)) if !text.is_empty() => return text,
                Ok(TaskOutcome::Failed { status, message }) => {
                    return format!("{title}: the app answered {status} {message}")
                        .trim_end()
                        .to_owned();
                }
                Ok(_) => return format!("{title} gave no form to open."),
                Err(err) => return format!("Could not open {title}: {err}"),
            },
            None => return format!("{title} has no form to open."),
        };
        let def = self.apps_definition(&app.id).or_else(|| {
            self.offline
                .load(Ordering::Relaxed)
                .then(|| fixtures::definition(&app.id))
                .flatten()
        });
        let (engine, app2, ext2, cmd2, conv2) = (
            self.clone(),
            app.clone(),
            ext.clone(),
            cmd.clone(),
            conv.to_owned(),
        );
        let on_submit = Arc::new(move |result: Value| {
            let (engine, app, ext, cmd, conv) = (
                engine.clone(),
                app2.clone(),
                ext2.clone(),
                cmd2.clone(),
                conv2.clone(),
            );
            tokio::spawn(async move {
                engine
                    .composer_task_submitted(&app, &ext, &cmd, &conv, result)
                    .await;
            });
        });
        if self.apps_open_task(def.as_ref(), &cmd.id, &url, &task_title, on_submit) {
            format!("Opened {title}.")
        } else if !url.starts_with("https://") {
            format!("{title} has no https link to open.")
        } else {
            format!("{title} cannot be opened: the app is not known.")
        }
    }

    /// Asks the bot of an action command for its task module.
    async fn ca_fetch_task(
        &self,
        app: &PlaceApp,
        ext: &Extension,
        cmd: &ExtensionCommand,
        conv: &str,
    ) -> teams_proto::Result<TaskOutcome> {
        if self.offline.load(Ordering::Relaxed) {
            return Ok(fixtures::task(&app.id, &cmd.id));
        }
        bots::fetch_task(
            &self.client,
            app,
            ext,
            cmd,
            CONTEXT,
            notes_free(conv),
            &self.my_name,
        )
        .await
    }

    /// The task module's page submitted `result` (`tasks.submitTask`, `dialog.url.submit`): it
    /// goes to the bot (`composeExtension/submitAction`) and the card the bot answers with is
    /// added to the draft of the chat, like a picked search result. Nothing is sent to the
    /// chat; the user sends the message. A page that closes without a result sends nothing.
    pub(crate) async fn composer_task_submitted(
        self: &Arc<Self>,
        app: &PlaceApp,
        ext: &Extension,
        cmd: &ExtensionCommand,
        conv: &str,
        result: Value,
    ) {
        if result.is_null() {
            return;
        }
        let answer = if self.offline.load(Ordering::Relaxed) {
            Ok(fixtures::submit(&app.id, &cmd.id, &result))
        } else {
            bots::submit_action(
                &self.client,
                app,
                ext,
                cmd,
                CONTEXT,
                &result,
                notes_free(conv),
                &self.my_name,
            )
            .await
        };
        let mut s = self.composer_apps.lock().unwrap();
        let note = match answer.map(submitted_cards) {
            Ok(Ok(items)) => {
                let mut note = String::new();
                for item in items {
                    note = add_card(&mut s.cards, conv, app.id.clone(), app.name.clone(), item);
                }
                note
            }
            Ok(Err(text)) => text,
            Err(err) => format!("{} failed: {err}", cmd.title),
        };
        s.note = note;
        drop(s);
        self.ca_push();
    }

    /// Takes the cards out of a chat's draft, for the send path, and clears them from the
    /// picker.
    pub(crate) fn take_composer_cards(self: &Arc<Self>, conv: &str) -> Vec<CardAttachment> {
        let cards = self
            .composer_apps
            .lock()
            .unwrap()
            .cards
            .remove(conv)
            .unwrap_or_default();
        if !cards.is_empty() {
            self.ca_push();
        }
        cards
    }

    /// Puts cards back at the front of a chat's draft, when the message they were taken for
    /// could not be queued.
    pub(crate) fn restore_composer_cards(
        self: &Arc<Self>,
        conv: &str,
        mut cards: Vec<CardAttachment>,
    ) {
        if cards.is_empty() {
            return;
        }
        let mut s = self.composer_apps.lock().unwrap();
        let list = s.cards.entry(conv.to_owned()).or_default();
        cards.append(list);
        *list = cards;
        drop(s);
        self.ca_push();
    }

    /// The cards in a chat's draft (tests).
    #[cfg(test)]
    pub(crate) fn composer_cards(&self, conv: &str) -> Vec<CardAttachment> {
        self.composer_apps
            .lock()
            .unwrap()
            .cards
            .get(conv)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_store::bots::{QueryResults, parse_query_response};

    fn demo_apps() -> Vec<PlaceApp> {
        bots::parse_place_apps(&fixtures::place_apps())
    }

    fn names(apps: &[PlaceApp]) -> Vec<&str> {
        apps.iter().map(|a| a.name.as_str()).collect()
    }

    #[test]
    fn commands_parse_with_their_arguments_and_refuse_bad_numbers() {
        use ComposerAppsCmd::*;
        assert_eq!(ComposerAppsCmd::parse("list", ""), Some(List));
        assert_eq!(
            ComposerAppsCmd::parse("query", "cat"),
            Some(Query("cat".into()))
        );
        assert_eq!(ComposerAppsCmd::parse("choose", "2"), Some(Choose(2)));
        assert_eq!(ComposerAppsCmd::parse("choose", "x"), None);
        assert_eq!(ComposerAppsCmd::parse("remove", "-1"), None);
        assert_eq!(ComposerAppsCmd::parse("nope", ""), None);
    }

    #[test]
    fn keys_round_trip_and_refuse_anything_else() {
        assert_eq!(parse_key(&key_text((1, 0, 12))), Some((1, 0, 12)));
        for bad in ["", "1", "1.2", "1.2.3.4", "a.b.c", "1..3", "-1.0.0"] {
            assert_eq!(parse_key(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_apps_with_a_compose_command_in_the_scope_are_listed() {
        let group = composer_apps(demo_apps(), scopes_for(ConvKind::Group));
        // Not Message Tools (message commands only), Helper Bot (a bot) or the app that is
        // only featured.
        assert_eq!(names(&group), ["GIF Search", "Contoso Tickets"]);
        let tickets = &group[1].extensions[0];
        let titles: Vec<_> = tickets.commands.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Find a ticket", "Create a ticket", "Report a problem"],
            "no message command"
        );

        let one_on_one = composer_apps(demo_apps(), scopes_for(ConvKind::OneOnOne));
        assert_eq!(
            names(&one_on_one),
            ["GIF Search"],
            "tickets work in teams and groups"
        );
        let channel = composer_apps(demo_apps(), scopes_for(ConvKind::Channel));
        assert_eq!(names(&channel), ["GIF Search", "Contoso Tickets"]);
    }

    #[test]
    fn the_command_list_groups_commands_under_their_app() {
        let apps = composer_apps(demo_apps(), scopes_for(ConvKind::Group));
        let rows = command_rows(&apps);
        let shape: Vec<_> = rows
            .iter()
            .map(|r| (r.header, r.title.as_str(), r.action))
            .collect();
        assert_eq!(
            shape,
            [
                (true, "", false),
                (false, "Search GIFs", false),
                (true, "", false),
                (false, "Find a ticket", false),
                (false, "Create a ticket", true),
                (false, "Report a problem", true),
            ]
        );
        assert_eq!(rows[1].key, "0.0.0");
        assert_eq!(rows[4].key, "1.0.1");
        assert_eq!(rows[3].app, "Contoso Tickets");
    }

    #[test]
    fn the_text_goes_to_the_first_parameter() {
        let apps = composer_apps(demo_apps(), scopes_for(ConvKind::Group));
        let search = &apps[0].extensions[0].commands[0];
        assert_eq!(
            query_params(search, "cat"),
            [("query".to_owned(), "cat".to_owned())]
        );
        let create = &apps[1].extensions[0].commands[1];
        assert!(
            query_params(create, "cat").is_empty(),
            "no field, no parameter"
        );
    }

    #[test]
    fn fixtures_filter_by_text_and_the_cards_map_to_previews() {
        let (all, status) = outcome_view(fixtures::query(fixtures::GIF_APP, "searchGifs", ""));
        assert_eq!(all.len(), 3);
        assert_eq!(status, "");
        let (cat, _) = outcome_view(fixtures::query(fixtures::GIF_APP, "searchGifs", " CAT "));
        assert_eq!(cat.len(), 1);
        let p = preview_of(&cat[0]);
        assert_eq!(
            (p.title.as_str(), p.subtitle.as_str()),
            ("Cat typing", "Cats")
        );
        assert_eq!(
            p.image.as_deref(),
            Some("https://gifs.example.invalid/cat.gif")
        );

        let (tickets, _) =
            outcome_view(fixtures::query(fixtures::TICKET_APP, "findTicket", "login"));
        let p = preview_of(&tickets[0]);
        assert_eq!(
            p.title, "TCK-101 Login fails",
            "an adaptive card has no title key"
        );
        assert_eq!(p.subtitle, "Open, assigned to Sam");

        let (none, status) = outcome_view(fixtures::query(fixtures::GIF_APP, "searchGifs", "zzz"));
        assert!(none.is_empty());
        assert_eq!(status, "No results.");
    }

    #[test]
    fn answers_that_are_not_results_become_a_line_of_text() {
        let answer = |kind, text: Option<&str>| {
            outcome_view(QueryOutcome::Results(QueryResults {
                kind,
                layout: None,
                text: text.map(str::to_owned),
                items: Vec::new(),
                actions: Vec::new(),
            }))
            .1
        };
        assert!(answer(ResultKind::Auth, None).contains("sign in"));
        assert!(answer(ResultKind::SilentAuth, None).contains("single sign-on"));
        assert!(answer(ResultKind::Config, None).contains("set up"));
        assert_eq!(
            answer(ResultKind::Message, Some("Nothing for that")),
            "Nothing for that"
        );
        assert_eq!(answer(ResultKind::Result, None), "No results.");
        let failed = outcome_view(QueryOutcome::Failed {
            status: 404,
            message: "ConversationNotFound".into(),
        });
        assert_eq!(failed.1, "The app answered 404: ConversationNotFound");
        assert_eq!(outcome_view(QueryOutcome::Empty).1, "No results.");
    }

    #[test]
    fn a_submit_answer_gives_its_cards_or_a_line_of_text() {
        // A result and a preview both carry cards.
        let some = fixtures::submit(fixtures::TICKET_APP, "createTicket", &serde_json::json!({}));
        assert_eq!(submitted_cards(some).unwrap().len(), 1);
        let preview = parse_query_response(&serde_json::json!({"composeExtension": {
            "type": "botMessagePreview",
            "activityPreview": {"attachments": [{
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": {"type": "AdaptiveCard"},
            }]},
        }}));
        assert_eq!(submitted_cards(preview).unwrap().len(), 1);
        // A bot that posts by itself answers nothing; a message is shown as it is.
        let none = "The app sent no card to add.";
        assert_eq!(submitted_cards(QueryOutcome::Empty).unwrap_err(), none);
        let message = parse_query_response(
            &serde_json::json!({"composeExtension": {"type": "message", "text": "Ticket denied"}}),
        );
        assert_eq!(submitted_cards(message).unwrap_err(), "Ticket denied");
        let empty =
            parse_query_response(&serde_json::json!({"composeExtension": {"type": "result"}}));
        assert_eq!(submitted_cards(empty).unwrap_err(), none);
        let failed = submitted_cards(QueryOutcome::Failed {
            status: 500,
            message: "boom".into(),
        });
        assert_eq!(failed.unwrap_err(), "The app answered 500: boom");
        // Another app's command gets nothing from the demo bot.
        let other = fixtures::submit(fixtures::GIF_APP, "searchGifs", &serde_json::json!({}));
        assert_eq!(submitted_cards(other).unwrap_err(), none);
    }

    #[test]
    fn the_notes_chat_is_asked_without_a_conversation() {
        assert_eq!(notes_free("48:notes"), "");
        assert_eq!(notes_free("19:x@thread.v2"), "19:x@thread.v2");
    }

    #[test]
    fn the_cards_of_a_draft_are_capped() {
        let item = fixtures::submit(fixtures::TICKET_APP, "createTicket", &serde_json::json!({}));
        let item = submitted_cards(item).unwrap().remove(0);
        let mut cards = HashMap::new();
        for _ in 0..MAX_CARDS {
            let note = add_card(&mut cards, "c", "a".into(), "A".into(), item.clone());
            assert!(note.starts_with("Added "), "{note}");
        }
        let note = add_card(&mut cards, "c", "a".into(), "A".into(), item);
        assert_eq!(note, format!("A message holds up to {MAX_CARDS} cards."));
        assert_eq!(cards["c"].len(), MAX_CARDS);
    }
}
