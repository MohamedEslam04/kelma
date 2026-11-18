//! Drives the real `AppWindow` through Slint's testing backend: keyboard input into the composer,
//! hovering a message and clicking its action bar. No display or network needed.

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::{self as testing, ElementHandle};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, ModelRc, SharedString, VecModel};

use crate::{AppWindow, Msg};

mod activity;

type Log = Rc<RefCell<Vec<String>>>;

fn app_with_messages(msgs: Vec<Msg>) -> AppWindow {
    testing::init_no_event_loop();
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
    assert_eq!(tb.len(), 4, "emoji, image, file, send");

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
