//! A chat the user is not a member of (a meeting they were invited to and have not joined): the
//! pane says so calmly, with no red bar, no messages and no composer, and the header keeps its
//! Join button. Raw HTTP answers never reach the banner.

use super::*;
use i_slint_backend_testing::AccessibleRole;
use store::{ConvKind, ConversationRow};

const MEETING: &str = "19:meeting_abc@thread.v2";
const MEETING_NOTE: &str =
    "You're not in this meeting chat yet. It becomes available after you join the meeting.";
const GROUP_NOTE: &str = "You're no longer a member of this conversation.";

fn conversation(id: &str, kind: ConvKind, title: &str) -> ConversationRow {
    ConversationRow {
        id: id.into(),
        kind,
        title: title.into(),
        team_id: None,
        team_name: None,
        last_activity_ms: teams_proto::time::now_ms(),
        last_preview: String::new(),
        unread: false,
        favorite: false,
        read_horizon_ms: 0,
    }
}

fn harness() -> Harness {
    let store = Store::open_in_memory().unwrap();
    demo::seed(&store, teams_proto::time::now_ms()).unwrap();
    store
        .upsert_conversation(&conversation(MEETING, ConvKind::Meeting, "Planning sync"))
        .unwrap();
    store
        .upsert_conversation(&conversation(
            "19:old@thread.v2",
            ConvKind::Group,
            "Old group",
        ))
        .unwrap();
    let h = Harness::with_store(store);
    crate::ui::call::wire(&h.app);
    h
}

fn composer_shown(app: &AppWindow) -> bool {
    ElementHandle::find_by_element_id(app, "Composer::composer-input")
        .next()
        .is_some()
}

fn join_buttons(app: &AppWindow) -> usize {
    find_label(app, "Join meeting")
        .into_iter()
        .filter(|e| e.accessible_role() == Some(AccessibleRole::Button))
        .count()
}

fn not_a_member() -> teams_proto::Error {
    teams_proto::Error::NotMember {
        context: "messages",
    }
}

#[test]
fn a_meeting_chat_the_user_is_not_in_shows_a_calm_note_and_no_composer() {
    let h = harness();
    h.engine.test_fail_history(not_a_member());
    h.select(MEETING);

    assert_eq!(find_label(&h.app, MEETING_NOTE).len(), 1);
    assert_eq!(h.app.get_banner(), "", "no error bar");
    assert!(!composer_shown(&h.app), "nothing to type into");
    assert_eq!(h.app.get_messages().row_count(), 0);
    // The header stays, with the way to join.
    assert_eq!(h.app.get_current_id(), MEETING);
    assert_eq!(join_buttons(&h.app), 1, "the Join button stays");

    // Another chat is normal again, and so is this one when it is opened anew.
    h.select(NOTES);
    assert!(find_label(&h.app, MEETING_NOTE).is_empty());
    assert!(composer_shown(&h.app));
    h.select(MEETING);
    assert!(find_label(&h.app, MEETING_NOTE).is_empty());
    assert!(composer_shown(&h.app));
}

#[test]
fn other_chats_say_the_user_is_no_longer_a_member() {
    let h = harness();
    h.engine.test_fail_history(not_a_member());
    h.select("19:old@thread.v2");

    assert_eq!(find_label(&h.app, GROUP_NOTE).len(), 1);
    assert!(find_label(&h.app, MEETING_NOTE).is_empty());
    assert_eq!(h.app.get_banner(), "");
    assert!(!composer_shown(&h.app));
    assert_eq!(join_buttons(&h.app), 0, "only a meeting chat has Join");
}

#[test]
fn opening_the_chat_again_asks_once_more() {
    let h = harness();
    h.engine.test_fail_history(not_a_member());
    h.select(MEETING);
    assert!(!composer_shown(&h.app));
    // The user joined in the meantime: the same click on the same chat reads it again.
    h.select(MEETING);
    assert!(find_label(&h.app, MEETING_NOTE).is_empty());
    assert!(composer_shown(&h.app));
}

#[test]
fn the_note_fits_a_narrow_window() {
    let h = harness();
    h.app.window().set_size(LogicalSize::new(760., 800.));
    h.engine.test_fail_history(not_a_member());
    h.select(MEETING);
    let note = &find_label(&h.app, MEETING_NOTE)[0];
    let (pos, size) = (note.absolute_position(), note.size());
    assert!(pos.x >= 0.0, "starts at {}", pos.x);
    assert!(
        pos.x + size.width <= 760.0,
        "ends at {}",
        pos.x + size.width
    );
}

#[test]
fn a_raw_http_answer_never_shows_in_the_banner() {
    let h = harness();
    let raw =
        r#"messages: HTTP 403: {"errorCode":209,"message":"{\"subCode\":\"AclCheckFailed\"}"}"#;
    UiHandle::new(h.app.as_weak()).banner(raw.into());
    let shown = h.app.get_banner().to_string();
    assert_eq!(shown, "Teams didn't allow that.");
    UiHandle::new(h.app.as_weak()).banner("Reminder set for 10:00".into());
    assert_eq!(h.app.get_banner(), "Reminder set for 10:00");
}
