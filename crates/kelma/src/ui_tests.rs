//! Drives the real `AppWindow` through Slint's testing backend: keyboard input into the composer,
//! hovering a message and clicking its action bar. No display or network needed.

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::{self as testing, ElementHandle};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, SharedString, VecModel,
};

use crate::backend::{Cmd, Engine};
use crate::ui::UiHandle;
use crate::{AppWindow, Msg, demo};
use std::sync::Arc;
use store::Store;
use teams_proto::Client;

type Log = Rc<RefCell<Vec<String>>>;

/// The testing platform is per thread and can be set once; several windows may share it.
fn init_platform() {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if !DONE.replace(true) {
        testing::init_no_event_loop();
    }
}

fn app_with_messages(msgs: Vec<Msg>) -> AppWindow {
    init_platform();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1200., 800.));
    app.set_login_state("ready".into());
    app.set_current_id("48:notes".into());
    app.set_current_title("Notes".into());
    app.set_messages(ModelRc::new(VecModel::from(msgs)));
    app.show().unwrap();
    app
}

fn msg(id: &str, text: &str, mine: bool) -> Msg {
    Msg {
        id: id.into(),
        sender: "Me".into(),
        initials: "ME".into(),
        mine,
        time: "10:00".into(),
        show_header: true,
        body: slint::StyledText::from_plain_text(text),
        plain: text.into(),
        ..Default::default()
    }
}

fn composer(app: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(app, "Composer::composer-input")
        .next()
        .expect("composer input")
}

fn key(app: &AppWindow, text: impl Into<SharedString>, pressed: bool) {
    let text = text.into();
    app.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    });
}

/// Types text the way a keyboard layout delivers it: one key event per character.
fn type_text(app: &AppWindow, text: &str) {
    for ch in text.chars() {
        key(app, ch, true);
        key(app, ch, false);
    }
}

fn enter(app: &AppWindow) {
    key(app, Key::Return, true);
    key(app, Key::Return, false);
}

fn record_send(app: &AppWindow) -> Log {
    let log = Log::default();
    let l = log.clone();
    app.on_send(move |t| l.borrow_mut().push(t.to_string()));
    log
}

#[test]
fn composer_takes_arabic_and_enter_sends() {
    let app = app_with_messages(vec![]);
    let sent = record_send(&app);
    composer(&app).mock_single_click(PointerEventButton::Left);

    type_text(&app, "مرحبا يا عالم 👋 hello");
    assert_eq!(app.get_draft(), "مرحبا يا عالم 👋 hello");

    enter(&app);
    assert_eq!(*sent.borrow(), vec!["مرحبا يا عالم 👋 hello".to_string()]);
    assert_eq!(app.get_draft(), "", "composer is cleared after send");
}

#[test]
fn shift_enter_adds_a_line_and_empty_enter_sends_nothing() {
    let app = app_with_messages(vec![]);
    let sent = record_send(&app);
    composer(&app).mock_single_click(PointerEventButton::Left);

    enter(&app);
    assert!(sent.borrow().is_empty(), "empty composer does not send");

    type_text(&app, "سطر");
    key(&app, Key::Shift, true);
    enter(&app);
    key(&app, Key::Shift, false);
    type_text(&app, "line");
    assert_eq!(app.get_draft(), "سطر\nline");
    assert!(sent.borrow().is_empty());
}

#[test]
fn typing_ticks_the_typing_indicator() {
    let app = app_with_messages(vec![]);
    let ticks = Rc::new(RefCell::new(0));
    let t = ticks.clone();
    app.on_typing_tick(move || *t.borrow_mut() += 1);
    composer(&app).mock_single_click(PointerEventButton::Left);
    type_text(&app, "ab");
    assert_eq!(*ticks.borrow(), 2);
}

fn hover(app: &AppWindow, row: &ElementHandle) {
    let p = row.absolute_position();
    let s = row.size();
    app.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(p.x + s.width / 2., p.y + s.height / 2.),
    });
}

fn rows(app: &AppWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_type_name(app, "MessageRow").collect()
}

/// Action-bar buttons (reply, edit, delete) of the hovered row, left to right.
fn bar_buttons(app: &AppWindow, row: &ElementHandle) -> Vec<ElementHandle> {
    let top = row.absolute_position().y;
    let mut v: Vec<_> = ElementHandle::find_by_element_id(app, "IconButton::touch")
        .filter(|b| {
            let y = b.absolute_position().y;
            y >= top && y < top + 32.
        })
        .collect();
    v.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
    v
}

#[test]
fn hover_shows_actions_and_they_fire_for_own_message() {
    let app = app_with_messages(vec![msg("m1", "first", false), msg("m2", "mine", true)]);
    let log = Log::default();
    {
        let l = log.clone();
        app.on_react(move |id, k| l.borrow_mut().push(format!("react {id} {k}")));
        let l = log.clone();
        app.on_reply(move |id| l.borrow_mut().push(format!("reply {id}")));
        let l = log.clone();
        app.on_edit(move |id| l.borrow_mut().push(format!("edit {id}")));
        let l = log.clone();
        app.on_delete(move |id| l.borrow_mut().push(format!("delete {id}")));
    }
    let rows = rows(&app);
    assert_eq!(rows.len(), 2);
    let own = &rows[1];

    assert_eq!(
        ElementHandle::find_by_element_id(&app, "MessageRow::et").count(),
        0,
        "no action bar before hover"
    );
    hover(&app, own);
    let reacts: Vec<_> = ElementHandle::find_by_element_id(&app, "MessageRow::et").collect();
    assert_eq!(reacts.len(), 6, "six quick reactions");
    reacts[1].mock_single_click(PointerEventButton::Left);

    let buttons = bar_buttons(&app, own);
    assert_eq!(buttons.len(), 3, "reply, edit, delete on own message");
    for b in &buttons {
        b.mock_single_click(PointerEventButton::Left);
    }
    assert_eq!(
        *log.borrow(),
        vec!["react m2 heart", "reply m2", "edit m2", "delete m2"]
    );
}

#[test]
fn others_messages_only_offer_react_and_reply() {
    let app = app_with_messages(vec![msg("m1", "theirs", false)]);
    let row = &rows(&app)[0];
    hover(&app, row);
    assert_eq!(bar_buttons(&app, row).len(), 1, "reply only");
}

#[test]
fn send_button_sends_draft() {
    let app = app_with_messages(vec![]);
    let sent = record_send(&app);
    app.set_draft(SharedString::from("from button"));
    let send = ElementHandle::find_by_element_id(&app, "IconButton::touch")
        .max_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x))
        .unwrap();
    send.mock_single_click(PointerEventButton::Left);
    assert_eq!(*sent.borrow(), vec!["from button".to_string()]);
}

/// Composer toolbar buttons below the input, left to right: emoji, image, file, send.
fn toolbar(app: &AppWindow) -> Vec<ElementHandle> {
    let input_y = composer(app).absolute_position().y;
    let mut v: Vec<_> = ElementHandle::find_by_element_id(app, "IconButton::touch")
        .filter(|b| b.absolute_position().y > input_y)
        .collect();
    v.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
    v
}

#[test]
fn toolbar_emoji_picker_and_attach_buttons() {
    let app = app_with_messages(vec![]);
    let log = Log::default();
    {
        let l = log.clone();
        app.on_attach_image(move || l.borrow_mut().push("image".into()));
        let l = log.clone();
        app.on_attach_file(move || l.borrow_mut().push("file".into()));
    }
    let tb = toolbar(&app);
    assert_eq!(tb.len(), 5, "emoji, image, file, more, send");

    app.set_draft("hi ".into());
    assert_eq!(
        ElementHandle::find_by_element_id(&app, "Composer::ea").count(),
        0
    );
    tb[0].mock_single_click(PointerEventButton::Left);
    let emoji: Vec<_> = ElementHandle::find_by_element_id(&app, "Composer::ea").collect();
    assert_eq!(emoji.len(), 32, "picker shows 32 emoji");
    emoji[9].mock_single_click(PointerEventButton::Left);
    assert_eq!(app.get_draft(), "hi 👍");

    tb[1].mock_single_click(PointerEventButton::Left);
    tb[2].mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), vec!["image", "file"]);
}

/// Buttons in the strip directly above the composer input (reply or edit preview).
fn strip_buttons(app: &AppWindow) -> Vec<ElementHandle> {
    let input_y = composer(app).absolute_position().y;
    ElementHandle::find_by_element_id(app, "IconButton::touch")
        .filter(|b| {
            let y = b.absolute_position().y;
            y < input_y && y > input_y - 80.
        })
        .collect()
}

#[test]
fn reply_strip_shows_and_cancels() {
    let app = app_with_messages(vec![]);
    let cancelled = Rc::new(RefCell::new(false));
    let c = cancelled.clone();
    app.on_cancel_reply(move || *c.borrow_mut() = true);
    assert!(strip_buttons(&app).is_empty(), "no strip without a reply");

    app.set_reply_author("Sara".into());
    app.set_reply_preview("original text".into());
    let close = strip_buttons(&app);
    assert_eq!(close.len(), 1, "reply strip has a close button");
    close[0].mock_single_click(PointerEventButton::Left);
    assert!(*cancelled.borrow());
}

fn combo(app: &AppWindow, modifier: Key, ch: char) {
    key(app, modifier, true);
    key(app, ch, true);
    key(app, ch, false);
    key(app, modifier, false);
}

#[test]
fn arabic_cursor_selection_copy_paste() {
    let app = app_with_messages(vec![]);
    composer(&app).mock_single_click(PointerEventButton::Left);
    type_text(&app, "مرحبا بكم");

    // Arrow keys step one character in logical order: two steps back from the end, then insert.
    for _ in 0..2 {
        key(&app, Key::LeftArrow, true);
        key(&app, Key::LeftArrow, false);
    }
    type_text(&app, "ـ");
    assert_eq!(app.get_draft(), "مرحبا بـكم");

    // Select all, copy, delete, paste twice, mixed with English.
    combo(&app, Key::Control, 'a');
    combo(&app, Key::Control, 'c');
    key(&app, Key::Backspace, true);
    key(&app, Key::Backspace, false);
    assert_eq!(app.get_draft(), "");
    combo(&app, Key::Control, 'v');
    type_text(&app, " and ");
    combo(&app, Key::Control, 'v');
    assert_eq!(app.get_draft(), "مرحبا بـكم and مرحبا بـكم");

    // Shift+arrows select across the Arabic word; typing replaces the selection.
    combo(&app, Key::Control, 'a');
    key(&app, Key::Backspace, true);
    key(&app, Key::Backspace, false);
    type_text(&app, "hi اهلا");
    key(&app, Key::Shift, true);
    for _ in 0..4 {
        key(&app, Key::LeftArrow, true);
        key(&app, Key::LeftArrow, false);
    }
    key(&app, Key::Shift, false);
    type_text(&app, "there");
    assert_eq!(app.get_draft(), "hi there");
}

fn ctrl_enter(app: &AppWindow) {
    key(app, Key::Control, true);
    enter(app);
    key(app, Key::Control, false);
}

#[test]
fn start_a_new_line_mode_sends_on_ctrl_enter() {
    let app = app_with_messages(vec![]);
    let sent = record_send(&app);
    composer(&app).mock_single_click(PointerEventButton::Left);
    assert!(app.get_enter_sends(), "Enter sends by default");

    app.set_enter_sends(false);
    type_text(&app, "one");
    enter(&app);
    type_text(&app, "two");
    assert!(
        sent.borrow().is_empty(),
        "Enter does not send in new-line mode"
    );
    assert_eq!(app.get_draft(), "one\ntwo");

    ctrl_enter(&app);
    assert_eq!(*sent.borrow(), vec!["one\ntwo".to_string()]);
    assert_eq!(app.get_draft(), "");

    app.set_enter_sends(true);
    type_text(&app, "x");
    enter(&app);
    assert_eq!(
        sent.borrow().len(),
        2,
        "Enter sends again once switched back"
    );
}

#[test]
fn composer_more_menu_switches_the_enter_behavior() {
    let app = app_with_messages(vec![]);
    let log = Log::default();
    {
        let l = log.clone();
        app.on_set_enter_sends(move |on| l.borrow_mut().push(format!("enter-sends {on}")));
    }
    let more = &toolbar(&app)[3];
    assert_eq!(
        ElementHandle::find_by_element_id(&app, "Composer::enter-opt").count(),
        0
    );
    more.mock_single_click(PointerEventButton::Left);
    let opts: Vec<_> = ElementHandle::find_by_element_id(&app, "Composer::enter-opt").collect();
    assert_eq!(opts.len(), 2, "send the message, start a new line");

    opts[1].mock_single_click(PointerEventButton::Left);
    assert!(!app.get_enter_sends());
    assert_eq!(*log.borrow(), vec!["enter-sends false"]);
}

/// The real engine behind the real window, driven synchronously: every callback runs its command to
/// completion on a current-thread runtime and then renders what the engine marked dirty.
struct Harness {
    app: AppWindow,
    engine: Arc<Engine>,
    rt: Rc<tokio::runtime::Runtime>,
}

const NOTES: &str = "48:notes";
const BACKEND: &str = "19:backend@thread.v2";

impl Harness {
    fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        demo::seed(&store, teams_proto::time::now_ms()).unwrap();
        Self::with_store(store)
    }

    fn with_store(store: Store) -> Self {
        init_platform();
        let app = AppWindow::new().unwrap();
        app.window().set_size(LogicalSize::new(1200., 800.));
        app.set_login_state("ready".into());
        app.show().unwrap();
        let rt = Rc::new(
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        );
        let engine = Engine::new(
            Client::offline("test").unwrap(),
            store,
            session::Config::default(),
            UiHandle::new(app.as_weak()),
            demo::ME.into(),
            "Me".into(),
        );
        // No network, so errors are not shown.
        engine
            .offline
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let h = Harness { app, engine, rt };
        let run = h.runner();
        h.app.on_select_chat({
            let run = run.clone();
            move |id| run(Cmd::SelectChat(id.into()))
        });
        h.app.on_set_filter({
            let run = run.clone();
            move |f| run(Cmd::SetFilter(f.into()))
        });
        h.app.on_reply({
            let run = run.clone();
            move |id| run(Cmd::Reply(id.into()))
        });
        h.app.on_draft_edited({
            let run = run.clone();
            move |t| run(Cmd::DraftEdited(t.into()))
        });
        h
    }

    fn runner(&self) -> Rc<dyn Fn(Cmd)> {
        let (engine, rt) = (self.engine.clone(), self.rt.clone());
        Rc::new(move |cmd| {
            rt.block_on(async {
                engine.clone().handle(cmd).await;
                engine.render_dirty();
            });
        })
    }

    fn select(&self, id: &str) {
        self.runner()(Cmd::SelectChat(id.into()));
    }

    fn type_in_composer(&self, text: &str) {
        composer(&self.app).mock_single_click(PointerEventButton::Left);
        type_text(&self.app, text);
    }

    /// Every chat list row: favorites, chats, then channels.
    fn chats(&self) -> Vec<crate::ChatRow> {
        let a = &self.app;
        let mut rows: Vec<_> = a.get_chats_favorites().iter().collect();
        rows.extend(a.get_chats_main().iter());
        rows.extend(a.get_chats_channels().iter());
        rows
    }
}

#[test]
fn draft_survives_switching_chats() {
    let h = Harness::new();
    h.select(NOTES);
    h.type_in_composer("remember the milk");

    h.select(BACKEND);
    assert_eq!(h.app.get_draft(), "", "another chat starts empty");
    h.type_in_composer("review please");

    h.select(NOTES);
    assert_eq!(h.app.get_draft(), "remember the milk");
    h.select(BACKEND);
    assert_eq!(h.app.get_draft(), "review please");
}

#[test]
fn draft_is_saved_after_a_pause_and_cleared_by_sending() {
    let h = Harness::new();
    h.select(NOTES);
    h.type_in_composer("half a thought");
    assert!(
        h.engine.stored_draft(NOTES).is_none(),
        "typing alone does not hit the database"
    );
    h.rt.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(700)).await });
    assert_eq!(h.engine.stored_draft(NOTES).unwrap().text, "half a thought");

    h.runner()(Cmd::Send("half a thought".into(), String::new()));
    assert!(
        h.engine.stored_draft(NOTES).is_none(),
        "sent text is no longer a draft"
    );
}

#[test]
fn draft_and_reply_target_survive_a_restart() {
    let db = std::env::temp_dir().join(format!("kelma-drafts-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let seed = |db: &std::path::Path| {
        let store = Store::open(db).unwrap();
        demo::seed(&store, teams_proto::time::now_ms()).unwrap();
        store
    };
    let target = {
        let h = Harness::with_store(seed(&db));
        h.select(BACKEND);
        let target = h.engine.latest_message_id(BACKEND).expect("demo message");
        h.runner()(Cmd::Reply(target.clone()));
        h.type_in_composer("on it");
        h.select(NOTES); // closing the app flushes the same way
        target
    };

    let h = Harness::with_store(Store::open(&db).unwrap());
    h.select(BACKEND);
    assert_eq!(h.app.get_draft(), "on it");
    assert!(!h.app.get_reply_preview().is_empty(), "reply bar is back");
    assert_eq!(h.engine.reply_target_id().as_deref(), Some(target.as_str()));
    let _ = std::fs::remove_file(&db);
}

#[test]
fn chat_list_marks_drafts_and_has_a_drafts_view() {
    let h = Harness::new();
    h.select(NOTES);
    h.type_in_composer("unsent");
    h.select(BACKEND);

    let notes = h.chats().into_iter().find(|c| c.id == NOTES).unwrap();
    assert!(notes.draft);
    assert_eq!(notes.preview, "unsent");
    let backend = h.chats().into_iter().find(|c| c.id == BACKEND).unwrap();
    assert!(
        !backend.draft,
        "chats without a draft keep their last message"
    );

    h.runner()(Cmd::SetFilter("drafts".into()));
    let ids: Vec<_> = h.chats().into_iter().map(|c| c.id.to_string()).collect();
    assert_eq!(ids, vec![NOTES.to_string()]);
}
