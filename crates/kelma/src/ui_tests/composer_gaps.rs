//! Composer basics Teams has: spell check with suggestions, the "+" menu's meeting actions and
//! the Stickers tab. The spell check uses a tiny in-memory dictionary, not the system's.

use super::*;
use crate::{CalStore, GifStore, PeopleStore, SettingsStore};
use std::time::Duration;

fn spelled_app() -> AppWindow {
    crate::spell::install(crate::spell::tests::speller());
    app_with_messages(vec![])
}

/// Typing pauses: the composer checks the draft and places the underlines.
fn pause() {
    testing::mock_elapsed_time(Duration::from_millis(600));
    testing::mock_elapsed_time(Duration::from_millis(16));
}

fn mark(app: &AppWindow, word: &str) -> Vec<ElementHandle> {
    find_label(app, &format!("Misspelled: {word}"))
}

#[test]
fn a_misspelled_word_is_underlined_once_typing_pauses_and_a_suggestion_replaces_it() {
    let app = spelled_app();
    composer(&app).mock_single_click(PointerEventButton::Left);
    type_text(&app, "hello wrold ");
    assert!(mark(&app, "wrold").is_empty(), "nothing while typing");
    pause();
    let m = mark(&app, "wrold");
    assert_eq!(m.len(), 1, "the misspelled word is marked");
    assert!(mark(&app, "hello").is_empty());
    // The mark sits on the word: after "hello " and about five letters wide.
    let input = composer(&app);
    let (x, w) = (m[0].absolute_position().x, m[0].size().width);
    assert!(x > input.absolute_position().x + 20., "x {x}");
    assert!(w > 15. && w < 120., "width {w}");
    assert!((m[0].absolute_position().y - input.absolute_position().y).abs() < 10.);

    // Right click: the suggestions; picking one replaces the word in place.
    m[0].mock_single_click(PointerEventButton::Right);
    assert_eq!(find_label(&app, "Add to dictionary").len(), 1);
    find_label(&app, "world")[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(app.get_draft(), "hello world ");
    pause();
    assert!(mark(&app, "wrold").is_empty());
    assert!(mark(&app, "world").is_empty());
}

#[test]
fn add_to_dictionary_clears_the_mark_and_editing_after_a_word_keeps_it() {
    let app = spelled_app();
    composer(&app).mock_single_click(PointerEventButton::Left);
    type_text(&app, "kelma team ");
    pause();
    assert_eq!(mark(&app, "kelma").len(), 1);
    // Typing after the word keeps its mark in place.
    type_text(&app, "x");
    assert_eq!(mark(&app, "kelma").len(), 1);
    mark(&app, "kelma")[0].mock_single_click(PointerEventButton::Right);
    find_label(&app, "Add to dictionary")[0].mock_single_click(PointerEventButton::Left);
    testing::mock_elapsed_time(Duration::from_millis(16));
    assert!(mark(&app, "kelma").is_empty());
}

#[test]
fn arabic_is_checked_too_and_the_setting_turns_it_off() {
    let app = spelled_app();
    composer(&app).mock_single_click(PointerEventButton::Left);
    type_text(&app, "مرحبا عالمم ");
    pause();
    let m = mark(&app, "عالمم");
    assert_eq!(m.len(), 1);
    assert!(m[0].size().width > 10.);
    assert!(mark(&app, "مرحبا").is_empty());

    app.global::<SettingsStore>().set_check_spelling(false);
    testing::mock_elapsed_time(Duration::from_millis(16));
    assert!(mark(&app, "عالمم").is_empty(), "Check spelling off");
}

#[test]
fn the_plus_menu_schedules_a_meeting_or_meets_now_for_the_chat() {
    let app = app_with_messages(vec![]);
    app.set_current_id("19:group@thread.v2".into());
    app.set_current_title("Design review".into());
    app.global::<PeopleStore>().set_header_group(true);
    let log = Log::default();
    let l = log.clone();
    app.on_schedule_meeting(move |id| l.borrow_mut().push(id.to_string()));

    find_label(&app, "Actions and apps")[0].invoke_accessible_default_action();
    find_label(&app, "Schedule a meeting")
        .into_iter()
        .last()
        .expect("the + menu lists it")
        .mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), vec!["19:group@thread.v2"]);

    find_label(&app, "Actions and apps")[0].invoke_accessible_default_action();
    find_label(&app, "Meet now")
        .into_iter()
        .last()
        .expect("the + menu lists it")
        .mock_single_click(PointerEventButton::Left);
    assert_eq!(app.get_view(), "calendar");
    let cal = app.global::<CalStore>();
    assert_eq!(cal.get_prompt_kind(), "meet-now");
    assert_eq!(cal.get_prompt_text1(), "Design review", "the chat's name");
}

#[test]
fn a_channel_has_no_meeting_actions_in_the_plus_menu() {
    let app = app_with_messages(vec![]);
    app.set_current_id("19:tech@thread.tacv2".into());
    app.set_current_is_channel(true);
    find_label(&app, "Actions and apps")[0].invoke_accessible_default_action();
    assert!(find_label(&app, "Meet now").is_empty());
}

#[test]
fn the_stickers_tab_lists_teams_stickers_and_a_pick_sends_one() {
    use crate::backend::GifCmd;
    let h = Harness::new();
    h.select(BACKEND);
    let run = h.runner();
    h.app.global::<GifStore>().on_command(move |n, a| {
        if let Some(c) = GifCmd::parse(&n, &a) {
            run(Cmd::Gif(c));
        }
    });
    find_label(&h.app, "GIF")[0].invoke_accessible_default_action();
    find_label(&h.app, "Stickers")[0].invoke_accessible_default_action();
    assert_eq!(h.app.global::<GifStore>().get_tab(), "sticker");
    let first = h.app.global::<GifStore>().get_items().row_data(0).unwrap();
    assert_eq!(first.title, "Hello There", "popular stickers first");
    testing::mock_elapsed_time(Duration::from_millis(16));
    find_label(&h.app, "Hello There")[0].invoke_accessible_default_action();

    let queued = h.engine.test_store(|s| s.outbox_for(BACKEND).unwrap());
    assert_eq!(queued.len(), 1);
    assert!(
        queued[0]
            .html
            .contains(r#"itemtype="http://schema.skype.com/Sticker""#),
        "{}",
        queued[0].html
    );
}
