//! Window state restore (plan 9.10) through Slint's testing backend: the saved size and the
//! maximized flag land on the window, and the last chat is found in the sidebar models.

use super::*;
use crate::window_state::{RestorePlan, apply_restore, restore_command, restore_plan};
use crate::{ChannelRow, ChatRow, TeamRow};
use session::window_state::WindowState;

#[test]
fn saved_size_is_applied_to_the_window() {
    let app = new_window();
    let mut s = WindowState::default();
    s.set_size(1111, 777);
    apply_restore(app.window(), &restore_plan(&s));
    let scale = app.window().scale_factor();
    let size = app.window().size();
    assert_eq!(size.width, (1111. * scale).round() as u32);
    assert_eq!(size.height, (777. * scale).round() as u32);
}

#[test]
fn a_first_run_keeps_the_size_the_ui_asks_for() {
    let app = new_window();
    let before = app.window().size();
    apply_restore(app.window(), &restore_plan(&WindowState::default()));
    assert_eq!(app.window().size(), before);
}

#[test]
fn a_maximized_state_only_maximizes_and_keeps_the_size() {
    let app = new_window();
    let mut s = WindowState::default();
    s.set_size(900, 600);
    s.set_maximized(true);
    let plan = restore_plan(&s);
    // The testing backend has no window manager (and lays the window out on its own), so only
    // the plan is checked: it maximizes and sets neither size nor position.
    assert_eq!(
        (plan.size, plan.position, plan.maximized),
        (None, None, true)
    );
    apply_restore(app.window(), &plan);
    // The geometry to go back to stays in the state.
    assert_eq!((s.width, s.height), (900, 600));
}

#[test]
fn a_plan_without_a_position_leaves_the_position_alone() {
    let app = new_window();
    let plan = RestorePlan {
        size: Some((800, 600)),
        position: None,
        maximized: false,
    };
    let before = app.window().position();
    apply_restore(app.window(), &plan);
    assert_eq!(app.window().position(), before);
}

fn chat_row(id: &str, title: &str) -> ChatRow {
    ChatRow {
        id: id.into(),
        title: title.into(),
        ..Default::default()
    }
}

#[test]
fn the_last_chat_is_found_in_the_sidebar_or_skipped() {
    let app = new_window();
    app.set_chats_favorites(ModelRc::new(VecModel::from(vec![chat_row(
        "19:fav", "Fav",
    )])));
    app.set_chats_main(ModelRc::new(VecModel::from(vec![chat_row(
        "19:main", "Main",
    )])));
    app.set_chats_channels(ModelRc::new(VecModel::from(vec![chat_row(
        "19:chan", "Pinned",
    )])));
    app.set_teams(ModelRc::new(VecModel::from(vec![TeamRow {
        id: "t1".into(),
        channels: ModelRc::new(VecModel::from(vec![ChannelRow {
            id: "19:team-chan".into(),
            name: "General".into(),
            unread: false,
            ..Default::default()
        }])),
        ..Default::default()
    }])));

    assert!(matches!(
        restore_command(&app, "19:fav"),
        Some(Cmd::SelectChat(id)) if id == "19:fav"
    ));
    assert!(matches!(
        restore_command(&app, "19:main"),
        Some(Cmd::SelectChat(id)) if id == "19:main"
    ));
    assert!(matches!(
        restore_command(&app, "19:chan"),
        Some(Cmd::SelectChannel(id, t)) if id == "19:chan" && t == "Pinned"
    ));
    assert!(matches!(
        restore_command(&app, "19:team-chan"),
        Some(Cmd::SelectChannel(id, t)) if id == "19:team-chan" && t == "General"
    ));
    // Gone or never there: skipped silently.
    assert!(restore_command(&app, "19:gone").is_none());
}
