//! Catch-up through the real window: the "Catch up" chip, Alt+Down / Alt+Up and the
//! "While you were away" card each send the select command for the right chat.

use super::*;
use crate::ChatRow;

fn unread_row(id: &str, title: &str, mention: bool) -> ChatRow {
    ChatRow {
        unread: true,
        mention,
        ..chat_row(id, title)
    }
}

/// Sara (read), Omar (unread), Lina (unread, mentions me), Hadi (unread); Sara is open.
fn app_with_unread() -> (AppWindow, Log) {
    let app = app_with_messages(vec![]);
    app.set_current_id("c1".into());
    app.set_chats_main(ModelRc::new(VecModel::from(vec![
        chat_row("c1", "Sara"),
        unread_row("c2", "Omar", false),
        unread_row("c3", "Lina", true),
        unread_row("c4", "Hadi", false),
    ])));
    let log = Log::default();
    let l = log.clone();
    app.on_select_chat(move |id| l.borrow_mut().push(id.to_string()));
    // Keys reach the window-level handler once the focus is inside the window.
    composer(&app).mock_single_click(PointerEventButton::Left);
    (app, log)
}

fn alt(app: &AppWindow, k: Key) {
    key(app, Key::Alt, true);
    key(app, k, true);
    key(app, k, false);
    key(app, Key::Alt, false);
}

#[test]
fn the_chip_opens_the_first_unread_chat_mentions_first() {
    let (app, log) = app_with_unread();
    // The pill and its text both carry the label.
    let chip = find_label(&app, "Catch up · 3");
    assert!(!chip.is_empty(), "a chip with the unread count");
    chip[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), vec!["c3"]);
}

#[test]
fn the_chip_is_hidden_when_nothing_is_unread() {
    let app = app_with_messages(vec![]);
    app.set_chats_main(ModelRc::new(VecModel::from(vec![chat_row("c1", "Sara")])));
    assert!(find_label(&app, "Catch up · 0").is_empty());
    assert!(find_label(&app, "Catch up").is_empty());
}

#[test]
fn alt_down_and_alt_up_walk_the_unread_chats() {
    let (app, log) = app_with_unread();
    alt(&app, Key::DownArrow);
    assert_eq!(*log.borrow(), vec!["c2"]);

    app.set_current_id("c2".into());
    alt(&app, Key::DownArrow);
    assert_eq!(*log.borrow(), vec!["c2", "c3"]);

    app.set_current_id("c4".into());
    alt(&app, Key::UpArrow);
    assert_eq!(*log.borrow(), vec!["c2", "c3", "c3"]);
}

#[test]
fn alt_down_works_from_another_view_and_brings_the_chat_list_back() {
    let (app, log) = app_with_unread();
    app.set_view("teams".into());
    alt(&app, Key::DownArrow);
    assert_eq!(*log.borrow(), vec!["c2"]);
    assert_eq!(app.get_view(), "chat");
}

#[test]
fn at_the_end_of_the_list_there_is_a_notice_and_no_wrap() {
    let (app, log) = app_with_unread();
    app.set_current_id("c4".into());
    assert!(app.get_banner().is_empty());
    alt(&app, Key::DownArrow);
    assert!(log.borrow().is_empty(), "no wrap-around");
    assert_eq!(app.get_banner(), "You're all caught up");
    assert!(app.get_banner_info(), "an info notice, not an error");

    app.set_banner("".into());
    app.set_current_id("c2".into());
    alt(&app, Key::UpArrow);
    assert!(log.borrow().is_empty());
    assert_eq!(app.get_banner(), "You're all caught up");
}

#[test]
fn the_away_card_waits_for_the_first_sync_and_starts_catching_up() {
    let (app, log) = app_with_unread();
    assert!(find_label(&app, "While you were away").is_empty());
    app.set_synced(true);
    assert!(!find_label(&app, "While you were away").is_empty());
    // No message counts are known, so it counts chats; one of them mentions me.
    assert!(!find_label(&app, "3 chats with new messages, 1 mention").is_empty());
    let start = find_label(&app, "Start catching up");
    assert!(!start.is_empty());
    start[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), vec!["c3"]);
}

#[test]
fn the_away_card_is_dismissed_for_the_session() {
    let (app, _log) = app_with_unread();
    app.set_synced(true);
    let close = find_label(&app, "Dismiss");
    assert!(!close.is_empty());
    close[0].mock_single_click(PointerEventButton::Left);
    assert!(app.get_away_dismissed());
    assert!(find_label(&app, "While you were away").is_empty());
    // More unread chats do not bring it back.
    app.set_chats_main(ModelRc::new(VecModel::from(vec![
        unread_row("c5", "Mona", false),
        unread_row("c6", "Zain", false),
    ])));
    assert!(find_label(&app, "While you were away").is_empty());
}
