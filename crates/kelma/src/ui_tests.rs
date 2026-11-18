//! Drives the real `AppWindow` through Slint's testing backend: keyboard input into the composer,
//! hovering a message and clicking its action bar. No display or network needed.

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_backend_testing::{self as testing, ElementHandle};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, SharedString, VecModel,
};

use crate::{AppWindow, Msg, ReactionItem};

mod activity;

type Log = Rc<RefCell<Vec<String>>>;

fn app_with_messages(msgs: Vec<Msg>) -> AppWindow {
    testing::init_no_event_loop();
    let app = AppWindow::new().unwrap();
    crate::ui::install_local(&app);
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

fn label(e: &ElementHandle) -> String {
    e.accessible_label().unwrap_or_default().to_string()
}

fn click(e: &ElementHandle) {
    e.mock_single_click(PointerEventButton::Left);
}

/// Hover-bar buttons (the IconButtons in the band of the hovered row), left to right.
fn bar_buttons(app: &AppWindow, row: &ElementHandle) -> Vec<ElementHandle> {
    let top = row.absolute_position().y;
    let mut v: Vec<_> = ElementHandle::find_by_element_type_name(app, "IconButton")
        .filter(|b| {
            let y = b.absolute_position().y;
            y >= top && y < top + 32.
        })
        .collect();
    v.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
    v
}

/// The four quick reactions of the hover bar, left to right.
fn quick_reactions(app: &AppWindow) -> Vec<ElementHandle> {
    let mut v: Vec<_> = ElementHandle::find_by_element_id(app, "MessageRow::et").collect();
    v.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
    v
}

/// Labels of the open message menu, top to bottom.
fn menu_labels(app: &AppWindow) -> Vec<String> {
    menu_items(app).iter().map(label).collect()
}

fn menu_items(app: &AppWindow) -> Vec<ElementHandle> {
    let mut v: Vec<_> = ElementHandle::find_by_element_type_name(app, "MenuItem").collect();
    v.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    v
}

fn menu_item(app: &AppWindow, name: &str) -> ElementHandle {
    menu_items(app)
        .into_iter()
        .find(|i| label(i) == name)
        .unwrap_or_else(|| panic!("menu item {name:?} in {:?}", menu_labels(app)))
}

/// Hovers a row and presses "More actions" in its bar.
fn open_menu(app: &AppWindow, row: &ElementHandle) {
    hover(app, row);
    let more = bar_buttons(app, row).pop().expect("bar buttons");
    assert_eq!(label(&more), "More actions");
    click(&more);
    assert!(app.get_menu_open());
}

/// Records every callback the menu, picker, forward dialog and multi-select raise.
fn record_actions(app: &AppWindow) -> Log {
    let log = Log::default();
    macro_rules! rec {
        ($on:ident, $name:literal, ||) => {{
            let l = log.clone();
            app.$on(move || l.borrow_mut().push($name.to_string()));
        }};
        ($on:ident, $name:literal, |$($a:ident),*|) => {{
            let l = log.clone();
            app.$on(move |$($a),*| {
                let args: Vec<String> = vec![$($a.to_string()),*];
                l.borrow_mut().push(format!("{} {}", $name, args.join(" ")).trim().to_string());
            });
        }};
    }
    rec!(on_react, "react", |id, k|);
    rec!(on_reply, "reply", |id|);
    rec!(on_edit, "edit", |id|);
    rec!(on_delete, "delete", |id|);
    rec!(on_copy_link, "copy-link", |id|);
    rec!(on_toggle_save, "save", |id|);
    rec!(on_pin, "pin", |id|);
    rec!(on_mark_unread, "unread", |id|);
    rec!(on_forward_begin, "forward-begin", |id|);
    rec!(on_forward_begin_selected, "forward-begin-selected", ||);
    rec!(on_forward_search, "forward-search", |q|);
    rec!(on_forward_toggle, "forward-toggle", |id|);
    rec!(on_forward_send, "forward-send", |note|);
    rec!(on_forward_cancel, "forward-cancel", ||);
    rec!(on_select_begin, "select-begin", |id|);
    rec!(on_toggle_select, "select-toggle", |id|);
    rec!(on_select_cancel, "select-cancel", ||);
    log
}

#[test]
fn hover_bar_order_for_own_message() {
    let app = app_with_messages(vec![msg("m1", "first", false), msg("m2", "mine", true)]);
    let log = record_actions(&app);
    let rows = rows(&app);
    assert_eq!(rows.len(), 2);
    let own = &rows[1];

    assert!(
        quick_reactions(&app).is_empty(),
        "no action bar before hover"
    );
    hover(&app, own);

    // 👍 ❤️ 😆 😮 first ...
    let reacts = quick_reactions(&app);
    assert_eq!(reacts.len(), 4, "four quick reactions");
    for r in &reacts {
        click(r);
    }
    assert_eq!(
        *log.borrow(),
        vec![
            "react m2 like",
            "react m2 heart",
            "react m2 laugh",
            "react m2 surprised"
        ]
    );

    // ... then More reactions, Reply with quote, Edit, "…".
    let buttons = bar_buttons(&app, own);
    let labels: Vec<_> = buttons.iter().map(label).collect();
    assert_eq!(
        labels,
        ["More reactions", "Reply with quote", "Edit", "More actions"]
    );
    let last_reaction_x = reacts.last().unwrap().absolute_position().x;
    assert!(buttons[0].absolute_position().x > last_reaction_x);

    log.borrow_mut().clear();
    click(&buttons[1]);
    click(&buttons[2]);
    assert_eq!(*log.borrow(), vec!["reply m2", "edit m2"]);
}

#[test]
fn others_messages_have_no_edit_in_the_bar() {
    let app = app_with_messages(vec![msg("m1", "theirs", false)]);
    let row = &rows(&app)[0];
    hover(&app, row);
    let labels: Vec<_> = bar_buttons(&app, row).iter().map(label).collect();
    assert_eq!(
        labels,
        ["More reactions", "Reply with quote", "More actions"]
    );
}

#[test]
fn more_reactions_picker_lists_teams_keys_and_filters_by_search() {
    let app = app_with_messages(vec![msg("m1", "theirs", false)]);
    let log = record_actions(&app);
    let row = &rows(&app)[0];
    hover(&app, row);
    click(&bar_buttons(&app, row)[0]);
    assert!(app.get_picker_open());
    assert_eq!(app.get_picker_msg_id(), "m1");

    let cells = || -> Vec<ElementHandle> {
        ElementHandle::find_by_element_id(&app, "ReactionPicker::cell").collect()
    };
    let mut all = cells();
    all.sort_by(|a, b| {
        let (pa, pb) = (a.absolute_position(), b.absolute_position());
        pa.y.total_cmp(&pb.y).then(pa.x.total_cmp(&pb.x))
    });
    assert!(all.len() >= 48, "a full grid, not an empty picker");
    let first: Vec<_> = all.iter().take(6).map(label).collect();
    assert_eq!(
        first,
        ["like", "heart", "laugh", "surprised", "sad", "angry"],
        "Teams reactions come first"
    );

    let input = ElementHandle::find_by_element_id(&app, "ReactionPicker::input")
        .next()
        .expect("search input");
    click(&input);
    type_text(&app, "fire");
    let found = cells();
    assert!(!found.is_empty() && found.len() < all.len());
    let fire = found.iter().find(|c| label(c) == "fire").expect("fire");
    click(fire);

    assert_eq!(*log.borrow(), vec!["react m1 1f525_fire"]);
    assert!(!app.get_picker_open(), "picking closes the picker");
}

#[test]
fn picker_finds_a_search_with_no_match() {
    let app = app_with_messages(vec![msg("m1", "theirs", false)]);
    let row = &rows(&app)[0];
    hover(&app, row);
    click(&bar_buttons(&app, row)[0]);
    let input = ElementHandle::find_by_element_id(&app, "ReactionPicker::input")
        .next()
        .unwrap();
    click(&input);
    type_text(&app, "zzzzqqq");
    assert_eq!(
        ElementHandle::find_by_element_id(&app, "ReactionPicker::cell").count(),
        0
    );
}

#[test]
fn menu_for_others_message() {
    let app = app_with_messages(vec![msg("m1", "theirs", false)]);
    open_menu(&app, &rows(&app)[0]);
    assert_eq!(
        menu_labels(&app),
        [
            "Reply with quote",
            "Forward",
            "Copy message link",
            "Save this message",
            "Pin for everyone",
            "Mark as unread",
        ]
    );
}

#[test]
fn menu_for_own_message_has_edit_and_delete_but_no_save() {
    let app = app_with_messages(vec![msg("m1", "mine", true)]);
    open_menu(&app, &rows(&app)[0]);
    assert_eq!(
        menu_labels(&app),
        [
            "Reply with quote",
            "Forward",
            "Copy message link",
            "Pin for everyone",
            "Mark as unread",
            "Edit",
            "Delete",
        ]
    );
}

#[test]
fn menu_in_a_channel_has_no_pin_and_a_saved_message_offers_unsave() {
    let app = app_with_messages(vec![Msg {
        saved: true,
        ..msg("m1", "theirs", false)
    }]);
    app.set_current_is_channel(true);
    open_menu(&app, &rows(&app)[0]);
    let labels = menu_labels(&app);
    assert!(
        !labels.contains(&"Pin for everyone".to_string()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"Unsave this message".to_string()),
        "{labels:?}"
    );
}

#[test]
fn right_click_opens_the_same_menu() {
    let app = app_with_messages(vec![msg("m1", "theirs", false), msg("m2", "mine", true)]);
    rows(&app)[0].mock_single_click(PointerEventButton::Right);
    assert!(app.get_menu_open());
    assert_eq!(app.get_menu_msg().id, "m1");
    let others = menu_labels(&app);
    app.set_menu_open(false);

    open_menu(&app, &rows(&app)[0]);
    assert_eq!(menu_labels(&app), others, "bar and right click agree");
}

#[test]
fn menu_actions_fire_with_the_message_id_and_close_the_menu() {
    let app = app_with_messages(vec![msg("m1", "theirs", false), msg("m2", "mine", true)]);
    let log = record_actions(&app);
    let (theirs, mine) = {
        let r = rows(&app);
        (r[0].clone(), r[1].clone())
    };
    for (row, item) in [
        (&theirs, "Reply with quote"),
        (&theirs, "Copy message link"),
        (&theirs, "Save this message"),
        (&theirs, "Pin for everyone"),
        (&theirs, "Mark as unread"),
        (&mine, "Edit"),
        (&mine, "Delete"),
    ] {
        open_menu(&app, row);
        click(&menu_item(&app, item));
        assert!(!app.get_menu_open(), "{item} closes the menu");
    }
    assert_eq!(
        *log.borrow(),
        vec![
            "reply m1",
            "copy-link m1",
            "save m1",
            "pin m1",
            "unread m1",
            "edit m2",
            "delete m2"
        ]
    );
}

fn recipient(id: &str, title: &str, picked: bool) -> crate::Recipient {
    crate::Recipient {
        id: id.into(),
        title: title.into(),
        initials: "XX".into(),
        picked,
        ..Default::default()
    }
}

fn recipients(v: Vec<crate::Recipient>) -> ModelRc<crate::Recipient> {
    ModelRc::new(VecModel::from(v))
}

fn result_rows(app: &AppWindow) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_id(app, "ForwardDialog::hit").collect()
}

#[test]
fn forward_dialog_flow() {
    let app = app_with_messages(vec![msg("m1", "hello there", false)]);
    let log = record_actions(&app);
    open_menu(&app, &rows(&app)[0]);

    // Forward has two choices: this message, or several.
    click(&menu_item(&app, "Forward"));
    assert_eq!(
        menu_labels(&app)
            .iter()
            .filter(|l| *l == "This message" || *l == "Multiple messages")
            .count(),
        2
    );
    click(&menu_item(&app, "This message"));
    assert!(app.get_forward_open());
    assert!(!app.get_menu_open());
    assert_eq!(*log.borrow(), vec!["forward-begin m1"]);
    assert_eq!(app.get_forward_count(), 1);
    assert_eq!(app.get_forward_preview_author(), "Me");
    assert_eq!(app.get_forward_preview_text(), "hello there");

    // The engine answers with the chats to pick from.
    app.set_forward_results(recipients(vec![
        recipient("c1", "Sara Hassan", false),
        recipient("c2", "Project room", false),
    ]));

    // Typing in "Add recipients" searches.
    let search = ElementHandle::find_by_element_id(&app, "ForwardDialog::recipient-input")
        .next()
        .expect("recipient input");
    click(&search);
    type_text(&app, "sa");
    assert_eq!(log.borrow().last().unwrap(), "forward-search sa");

    // Forward stays disabled until someone is picked.
    let send = ElementHandle::find_by_element_id(&app, "ForwardDialog::send-touch")
        .next()
        .unwrap();
    click(&send);
    assert!(
        !log.borrow().iter().any(|l| l.starts_with("forward-send")),
        "nothing picked, nothing sent"
    );

    let rows = result_rows(&app);
    assert_eq!(rows.len(), 2);
    click(&rows[0]);
    assert_eq!(log.borrow().last().unwrap(), "forward-toggle c1");
    app.set_forward_picked(recipients(vec![recipient("c1", "Sara Hassan", true)]));
    app.set_forward_results(recipients(vec![
        recipient("c1", "Sara Hassan", true),
        recipient("c2", "Project room", false),
    ]));
    assert_eq!(result_rows(&app).len(), 2, "rows survive a refresh");

    let note = ElementHandle::find_by_element_id(&app, "ForwardDialog::note")
        .next()
        .expect("optional message");
    click(&note);
    type_text(&app, "FYI");
    click(&send);
    assert_eq!(log.borrow().last().unwrap(), "forward-send FYI");

    // Cancel closes and tells the engine.
    let cancel = ElementHandle::find_by_element_id(&app, "ForwardDialog::cancel-touch")
        .next()
        .unwrap();
    click(&cancel);
    assert!(!app.get_forward_open());
    assert_eq!(log.borrow().last().unwrap(), "forward-cancel");
}

fn checkboxes(app: &AppWindow) -> Vec<ElementHandle> {
    let mut v: Vec<_> = ElementHandle::find_by_accessible_label(app, "Select message").collect();
    v.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    v
}

#[test]
fn multi_select_ticks_messages_and_forwards_them() {
    let app = app_with_messages(vec![
        msg("m1", "one", false),
        msg("m2", "two", false),
        msg("m3", "three", true),
    ]);
    let log = record_actions(&app);
    assert!(
        checkboxes(&app).is_empty(),
        "no checkboxes outside the mode"
    );

    open_menu(&app, &rows(&app)[1]);
    click(&menu_item(&app, "Forward"));
    click(&menu_item(&app, "Multiple messages"));
    assert!(app.get_select_mode());
    assert_eq!(*log.borrow(), vec!["select-begin m2"]);
    assert_eq!(checkboxes(&app).len(), 3, "a checkbox per message");

    // No hover bar while selecting.
    hover(&app, &rows(&app)[0]);
    assert!(quick_reactions(&app).is_empty());

    // The engine owns the ticks; the row only reports the click.
    click(&rows(&app)[0]);
    click(&rows(&app)[2]);
    assert_eq!(log.borrow()[1..], ["select-toggle m1", "select-toggle m3"]);

    // The engine pushes the new state back.
    let model = app.get_messages();
    for i in 0..model.row_count() {
        let mut m = model.row_data(i).unwrap();
        m.selected = i != 1;
        model.set_row_data(i, m);
    }
    app.set_select_count(2);
    let ticked: Vec<_> = checkboxes(&app)
        .iter()
        .map(|c| c.accessible_checked().unwrap())
        .collect();
    assert_eq!(ticked, [true, false, true]);

    let forward = ElementHandle::find_by_element_id(&app, "SelectBar::forward-touch")
        .next()
        .expect("select bar");
    click(&forward);
    assert_eq!(log.borrow().last().unwrap(), "forward-begin-selected");
    assert!(app.get_forward_open());
    assert_eq!(app.get_forward_count(), 2);

    // Leaving the mode.
    app.set_forward_open(false);
    let cancel = ElementHandle::find_by_element_id(&app, "SelectBar::cancel-touch")
        .next()
        .unwrap();
    click(&cancel);
    assert!(!app.get_select_mode());
    assert_eq!(log.borrow().last().unwrap(), "select-cancel");
    assert!(checkboxes(&app).is_empty());
}

#[test]
fn select_bar_forward_is_disabled_without_ticks() {
    let app = app_with_messages(vec![msg("m1", "one", false)]);
    let log = record_actions(&app);
    app.set_select_mode(true);
    app.set_select_count(0);
    let forward = ElementHandle::find_by_element_id(&app, "SelectBar::forward-touch")
        .next()
        .unwrap();
    click(&forward);
    assert!(!app.get_forward_open());
    assert!(log.borrow().is_empty());
}

#[test]
fn reaction_chip_tooltip_lists_who_reacted() {
    let mut m = msg("m1", "react to me", false);
    m.reactions = ModelRc::new(VecModel::from(vec![ReactionItem {
        key: "like".into(),
        emoji: "👍".into(),
        count: 2,
        mine: true,
        names: "Sara Hassan, You".into(),
    }]));
    let app = app_with_messages(vec![m]);
    assert_eq!(
        ElementHandle::find_by_element_id(&app, "ReactionChip::tip").count(),
        0
    );
    let chip = ElementHandle::find_by_element_type_name(&app, "ReactionChip")
        .next()
        .expect("reaction chip");
    hover(&app, &chip);
    let tip = ElementHandle::find_by_element_id(&app, "ReactionChip::tip")
        .next()
        .expect("tooltip while hovering");
    assert_eq!(label(&tip), "Sara Hassan, You");
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

// ---- unread handling (plans/chat.md 5.6) ---------------------------------

fn find_label(app: &AppWindow, text: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, text).collect()
}

#[test]
fn new_messages_divider_sits_above_the_first_unread_row() {
    let mut unread = msg("m2", "first unread", false);
    unread.new_above = true;
    let app = app_with_messages(vec![
        msg("m1", "read", false),
        unread,
        msg("m3", "later", false),
    ]);
    let divider = find_label(&app, "New messages");
    assert_eq!(divider.len(), 1, "exactly one divider");
    let rows = rows(&app);
    let y = divider[0].absolute_position().y;
    assert!(y > rows[0].absolute_position().y, "below the read row");
    assert!(
        y < rows[1].absolute_position().y + 20.,
        "inside the unread row"
    );
    assert!(y < rows[2].absolute_position().y, "above the later row");
}

#[test]
fn no_divider_when_nothing_is_marked() {
    let app = app_with_messages(vec![msg("m1", "a", false), msg("m2", "b", false)]);
    assert!(find_label(&app, "New messages").is_empty());
}

#[test]
fn jump_button_shows_when_scrolled_up_with_the_count_and_scrolls_down() {
    let app = app_with_messages(vec![msg("m1", "a", false), msg("m2", "b", false)]);
    let jumps = Rc::new(RefCell::new(0));
    let j = jumps.clone();
    app.on_jump_to_newest(move || *j.borrow_mut() += 1);

    assert!(
        find_label(&app, "↓ Jump to newest").is_empty(),
        "hidden at the bottom"
    );

    app.set_at_bottom(false);
    let button = find_label(&app, "↓ Jump to newest");
    assert_eq!(button.len(), 1, "shown when scrolled up");

    app.set_new_count(3);
    let button = find_label(&app, "↓ Jump to newest · 3");
    assert_eq!(button.len(), 1, "count of new messages");

    let ticks = app.get_scroll_tick();
    button[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(*jumps.borrow(), 1);
    assert_eq!(
        app.get_scroll_tick(),
        ticks + 1,
        "asks the list to scroll down"
    );
}

#[test]
fn shift_escape_marks_everything_read_and_plain_escape_does_not() {
    let app = app_with_messages(vec![]);
    let fired = Rc::new(RefCell::new(0));
    let f = fired.clone();
    app.on_mark_all_read(move || *f.borrow_mut() += 1);
    composer(&app).mock_single_click(PointerEventButton::Left);

    key(&app, Key::Escape, true);
    key(&app, Key::Escape, false);
    assert_eq!(*fired.borrow(), 0);

    key(&app, Key::Shift, true);
    key(&app, Key::Escape, true);
    key(&app, Key::Escape, false);
    key(&app, Key::Shift, false);
    assert_eq!(*fired.borrow(), 1);
}

fn chat_row(id: &str, title: &str) -> crate::ChatRow {
    crate::ChatRow {
        id: id.into(),
        title: title.into(),
        section: "chats".into(),
        ..Default::default()
    }
}

#[test]
fn chat_list_menus_fire_mark_unread_and_mark_all_read() {
    let app = app_with_messages(vec![]);
    app.set_chats_main(ModelRc::new(VecModel::from(vec![
        chat_row("c1", "Sara"),
        chat_row("c2", "Omar"),
    ])));
    let log = Log::default();
    {
        let l = log.clone();
        app.on_mark_chat_unread(move |id| l.borrow_mut().push(format!("unread {id}")));
        let l = log.clone();
        app.on_mark_all_read(move || l.borrow_mut().push("all".into()));
    }

    // Right click on the second chat opens its menu.
    let item = find_label(&app, "Omar");
    assert_eq!(item.len(), 1);
    let p = item[0].absolute_position();
    let s = item[0].size();
    let at = LogicalPosition::new(p.x + s.width / 2., p.y + s.height / 2.);
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Right,
    });
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Right,
    });
    let entry = find_label(&app, "Mark as unread");
    assert_eq!(entry.len(), 1, "menu open");
    entry[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), vec!["unread c2"]);

    // The "…" button of the list offers Mark all as read.
    let more = ElementHandle::find_by_element_id(&app, "IconButton::touch")
        .find(|b| b.absolute_position().x < 400. && b.absolute_position().y < 120.)
        .expect("list menu button");
    more.mock_single_click(PointerEventButton::Left);
    let all = find_label(&app, "Mark all as read");
    assert_eq!(all.len(), 1);
    all[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), vec!["unread c2", "all"]);
}

#[test]
fn opening_at_the_divider_puts_it_near_the_top_of_a_long_list() {
    let msgs: Vec<Msg> = (0..160)
        .map(|i| {
            let text = if i % 3 == 0 {
                "a much longer message that wraps over several lines because it has a great deal to say about nothing in particular ".repeat(3)
            } else {
                "short".to_string()
            };
            let mut m = msg(&format!("m{i}"), &text, false);
            m.new_above = i == 110;
            m
        })
        .collect();
    let app = app_with_messages(msgs);
    app.set_divider_index(110);
    app.set_divider_tick(app.get_divider_tick() + 1);
    // Let the list lay out the rows around the new position and correct itself.
    for _ in 0..6 {
        testing::mock_elapsed_time(std::time::Duration::from_millis(50));
    }
    let divider = find_label(&app, "New messages");
    assert_eq!(divider.len(), 1, "the divider row is on screen");
    let y = divider[0].absolute_position().y;
    // The list starts below the 48px top bar and the 60px conversation header.
    assert!((108. ..108. + 120.).contains(&y), "divider at y={y}");
}

// ---- failed sends (plans/chat.md 5.8) -----------------------------------

fn outgoing(id: &str, text: &str, pending: bool, failed: bool) -> Msg {
    Msg {
        pending,
        failed,
        ..msg(id, text, true)
    }
}

#[test]
fn failed_row_offers_retry_and_delete_and_they_fire() {
    let app = app_with_messages(vec![
        msg("m1", "sent earlier", true),
        outgoing("pending-77", "did not go", false, true),
    ]);
    let log = Log::default();
    {
        let l = log.clone();
        app.on_retry_send(move |id| l.borrow_mut().push(format!("retry {id}")));
        let l = log.clone();
        app.on_discard_send(move |id| l.borrow_mut().push(format!("discard {id}")));
        let l = log.clone();
        app.on_delete(move |id| l.borrow_mut().push(format!("server-delete {id}")));
    }
    assert_eq!(
        find_label(&app, "Failed to send").len(),
        1,
        "only the failed row says so"
    );
    let retry = find_label(&app, "Retry");
    let delete = find_label(&app, "Delete");
    assert_eq!((retry.len(), delete.len()), (1, 1));
    retry[0].mock_single_click(PointerEventButton::Left);
    delete[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(
        *log.borrow(),
        vec!["retry pending-77", "discard pending-77"],
        "the failed row's Delete discards locally, it never deletes on the server"
    );
}

#[test]
fn pending_row_has_no_failure_actions_and_no_hover_bar() {
    let app = app_with_messages(vec![outgoing("pending-1", "sending", true, false)]);
    assert!(find_label(&app, "Failed to send").is_empty());
    assert!(find_label(&app, "Retry").is_empty());
    let row = &rows(&app)[0];
    hover(&app, row);
    assert_eq!(
        ElementHandle::find_by_element_id(&app, "MessageRow::et").count(),
        0,
        "no reactions on a message that is not sent yet"
    );
}
