//! UI-thread side of the top bar: the search box's dropdown (suggestions, recent searches, slash
//! commands) and Back and Forward. Fills the `TopBarStore` global from `crate::topbar` and turns
//! what the user picks into the same calls the rest of the window already makes.
//!
//! Commands from `TopBarStore.command(name, argument)`: input, focus, close, activate, submit,
//! back, forward, visit (`view` U+001F `chat`), help.

use super::{UiHandle, model, rgb};
use crate::suggest::SuggestData;
use crate::topbar::{self, Action, History, Pick, Recents, Row};
use crate::{AppWindow, FindStore, SettingsStore, TopBarStore, TopRow};
use slint::{ComponentHandle, SharedString};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

struct Ctl {
    recents: Recents,
    /// Where the recent searches are kept; none keeps them in memory only.
    dir: Option<PathBuf>,
    history: History,
    rows: Vec<Row>,
    suggestions: Vec<SuggestData>,
    ui: UiHandle,
}

type Shared = Rc<RefCell<Ctl>>;

thread_local! {
    static CTL: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

fn ctl() -> Option<Shared> {
    CTL.with(|c| c.borrow().clone())
}

fn s(v: impl Into<SharedString>) -> SharedString {
    v.into()
}

/// Wires the top bar. `config_dir` is where the recent searches live (`None` for tests that do
/// not need them kept).
pub fn install(window: &AppWindow, config_dir: Option<PathBuf>) {
    let recents = config_dir.as_deref().map(Recents::load).unwrap_or_default();
    let shared: Shared = Rc::new(RefCell::new(Ctl {
        recents,
        dir: config_dir,
        history: History::default(),
        rows: Vec::new(),
        suggestions: Vec::new(),
        ui: UiHandle::new(window.as_weak()),
    }));
    CTL.with(|c| *c.borrow_mut() = Some(shared.clone()));
    let weak = window.as_weak();
    window.global::<TopBarStore>().on_command(move |name, arg| {
        let Some(w) = weak.upgrade() else { return };
        // A call from inside a call (a callback fired while a command still runs) is dropped.
        let ask = match shared.try_borrow_mut() {
            Ok(mut c) => command(&mut c, &w, &name, &arg),
            Err(_) => None,
        };
        // Asked once the controller is free again: an engine that answers at once (tests)
        // calls back into it.
        if let Some(q) = ask {
            w.global::<FindStore>().invoke_command(s("suggest"), s(q));
        }
    });
}

fn to_row(r: &Row) -> TopRow {
    TopRow {
        kind: s(r.kind),
        id: s(r.id.as_str()),
        title: s(r.title.as_str()),
        subtitle: s(r.subtitle.as_str()),
        initials: s(r.initials.as_str()),
        color: rgb(r.color),
        section: s(r.section),
        insert: s(r.insert.as_str()),
    }
}

/// Shows the rows for what is in the box.
fn rebuild(w: &AppWindow, c: &mut Ctl) {
    let store = w.global::<TopBarStore>();
    c.rows = topbar::rows(&store.get_input(), c.recents.items(), &c.suggestions);
    store.set_rows(model(c.rows.iter().map(to_row).collect()));
    store.set_selected(-1);
}

fn save_recents(c: &Ctl) {
    if let Some(dir) = &c.dir
        && let Err(e) = c.recents.save(dir)
    {
        tracing::warn!(%e, "recent searches not saved");
    }
}

/// Empties the box, closes the dropdown and lifts the chat list filter.
fn clear_input(w: &AppWindow, c: &mut Ctl) {
    let store = w.global::<TopBarStore>();
    store.set_input(s(""));
    store.set_search_open(false);
    c.suggestions.clear();
    rebuild(w, c);
    w.invoke_search(s(""));
}

fn show_flags(w: &AppWindow, c: &Ctl) {
    let store = w.global::<TopBarStore>();
    store.set_can_back(c.history.can_back());
    store.set_can_forward(c.history.can_forward());
}

fn status_label(wire: &str) -> &'static str {
    match wire {
        "Available" => "Available",
        "Busy" => "Busy",
        "DoNotDisturb" => "Do not disturb",
        "BeRightBack" => "Be right back",
        "Away" => "Appear away",
        _ => "Appear offline",
    }
}

/// Opens a quick view (`saved`, `mentions`) over the chat list's conversation pane.
fn open_quick(w: &AppWindow, kind: &str) {
    w.set_view(s("chat"));
    w.global::<FindStore>().invoke_command(s("quick"), s(kind));
}

/// Runs a typed command. `true` when it did something, so the box can be emptied.
fn run(w: &AppWindow, c: &Ctl, action: Action) -> bool {
    match action {
        Action::Status(wire) => {
            let g = w.global::<SettingsStore>();
            // A command sets the status until it is changed, not for the last chosen duration.
            g.set_status_duration(s("never"));
            g.invoke_command(s("status"), s(wire));
            c.ui.info(format!("Status: {}", status_label(wire)));
        }
        Action::Unread => {
            w.set_view(s("chat"));
            w.set_filter(s("unread"));
            w.invoke_set_filter(s("unread"));
        }
        Action::Saved => open_quick(w, "saved"),
        Action::Mentions => open_quick(w, "mentions"),
        Action::Activity => w.invoke_navigate(s("activity")),
        Action::Files => w.invoke_navigate(s("onedrive")),
        Action::Help => w.invoke_open_url(s(topbar::HELP_URL)),
        Action::Keys => w.set_shortcuts_open(true),
        Action::Go(kind, name) => {
            w.global::<FindStore>()
                .invoke_command(s("go"), s(format!("{kind}\u{1f}{name}")));
        }
        Action::NeedsName(kind) => {
            c.ui.info(format!("Type a name after /{kind}."));
            return false;
        }
        Action::Unknown(word) => {
            c.ui.info(format!(
                "There is no command /{word}. Type / to see the list."
            ));
            return false;
        }
    }
    true
}

/// Opens the Search page for `text`.
fn search(w: &AppWindow, c: &mut Ctl, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    c.recents.add(text);
    save_recents(c);
    let f = w.global::<FindStore>();
    f.set_query(s(text));
    w.set_view(s("search"));
    f.invoke_command(s("search"), s(text));
    w.global::<TopBarStore>().set_search_open(false);
    rebuild(w, c);
}

fn apply(w: &AppWindow, c: &mut Ctl, pick: Pick, input: &str) {
    match pick {
        Pick::Insert(text) => {
            w.global::<TopBarStore>().set_input(s(text.as_str()));
            c.suggestions.clear();
            rebuild(w, c);
        }
        Pick::Search(text) => search(w, c, &text),
        Pick::Open(kind, id, title) => {
            c.recents.add(input);
            save_recents(c);
            w.global::<FindStore>().set_quick_open(false);
            if kind == "channel" {
                w.set_view(s("teams"));
                w.invoke_select_channel(s(id), s(title));
            } else {
                w.set_view(s("chat"));
                w.invoke_select_chat(s(id));
            }
            clear_input(w, c);
        }
        Pick::Run(action) => {
            if run(w, c, action) {
                clear_input(w, c);
            }
        }
        Pick::ClearRecents => {
            c.recents.clear();
            save_recents(c);
            rebuild(w, c);
        }
    }
}

/// Goes to a place of the history.
fn go(w: &AppWindow, place: &topbar::Place) {
    w.invoke_navigate(s(place.view.as_str()));
    if !place.chat.is_empty() && w.get_current_id() != place.chat.as_str() {
        w.invoke_select_chat(s(place.chat.as_str()));
    }
}

/// Runs one command; the text to ask the engine suggestions for, if any.
fn command(c: &mut Ctl, w: &AppWindow, name: &str, arg: &str) -> Option<String> {
    let store = w.global::<TopBarStore>();
    let mut ask = None;
    match name {
        "input" => {
            let q = arg.trim();
            // As in Teams, the box searches (its dropdown, then the Search page) and never
            // filters the chat list: a word left in the box once hid most chats and left the
            // user's sections empty with nothing saying why.
            c.suggestions.clear();
            if !q.is_empty() && !q.starts_with('/') {
                ask = Some(q.to_string());
            }
            store.set_search_open(true);
            rebuild(w, c);
        }
        "focus" => {
            store.set_search_open(true);
            rebuild(w, c);
        }
        "close" => store.set_search_open(false),
        "help" => w.invoke_open_url(s(topbar::HELP_URL)),
        "activate" => {
            let input = store.get_input().to_string();
            if let Some(row) = arg
                .parse::<usize>()
                .ok()
                .and_then(|i| c.rows.get(i))
                .cloned()
            {
                let pick = topbar::pick(&row, &input);
                apply(w, c, pick, &input);
            }
        }
        // Enter with nothing chosen in the list: a command runs, anything else is searched.
        "submit" => {
            let text = arg.trim();
            match topbar::parse(text) {
                Some(action) => apply(w, c, Pick::Run(action), text),
                None => search(w, c, text),
            }
        }
        "back" | "forward" => {
            let chat = w.get_current_id().to_string();
            let place = if name == "back" {
                c.history.back(&chat)
            } else {
                c.history.forward(&chat)
            };
            if let Some(p) = place {
                go(w, &p);
            }
            show_flags(w, c);
        }
        "visit" => {
            let (view, chat) = arg.split_once('\u{1f}').unwrap_or((arg, ""));
            c.history.visit(view, chat);
            show_flags(w, c);
        }
        other => tracing::debug!(command = other, "topbar: unknown command"),
    }
    ask
}

impl UiHandle {
    /// The engine's suggestions for `query`; dropped when the box has moved on.
    pub fn topbar_suggestions(&self, query: String, rows: Vec<SuggestData>) {
        self.run(move |w| {
            let Some(shared) = ctl() else { return };
            let Ok(mut c) = shared.try_borrow_mut() else {
                return;
            };
            if w.global::<TopBarStore>().get_input().trim() == query.trim() {
                c.suggestions = rows;
                rebuild(&w, &mut c);
            }
        });
    }
}
