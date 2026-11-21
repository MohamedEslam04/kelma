//! A chat list row never shows an empty title: a group chat nobody named lists its members, a
//! channel without a name is "General". The conversation header draws the list's glyph for a
//! meeting chat (calendar) and a group chat (people). Driven through the real window.

use super::*;
use store::{ConvKind, ConversationRow, UserRow};

const GROUP: &str = "19:unnamed@thread.v2";
const BARE_GROUP: &str = "19:bare@thread.v2";
const CHANNEL: &str = "19:nameless@thread.tacv2";
const MEETING: &str = "19:meeting_sprint@thread.v2";

fn row(id: &str, kind: ConvKind, title: &str) -> ConversationRow {
    ConversationRow {
        id: id.into(),
        kind,
        title: title.into(),
        team_id: None,
        team_name: matches!(kind, ConvKind::Channel).then(|| "Orchid".into()),
        last_activity_ms: teams_proto::time::now_ms() - 60_000,
        last_preview: "See you there".into(),
        unread: false,
        favorite: false,
        read_horizon_ms: 0,
    }
}

fn titles(app: &AppWindow) -> std::collections::HashMap<String, String> {
    [
        app.get_chats_favorites(),
        app.get_chats_main(),
        app.get_chats_channels(),
    ]
    .iter()
    .flat_map(|m| m.iter())
    .map(|c| (c.id.to_string(), c.title.to_string()))
    .collect()
}

fn harness() -> Harness {
    let store = Store::open_in_memory().unwrap();
    demo::seed(&store, teams_proto::time::now_ms()).unwrap();
    for (mri, name) in [("8:orgid:m1", "Ann"), ("8:orgid:m2", "Bo")] {
        store
            .upsert_user(&UserRow {
                mri: mri.into(),
                display_name: name.into(),
                email: None,
                avatar: None,
                updated_ms: 0,
            })
            .unwrap();
    }
    for r in [
        row(GROUP, ConvKind::Group, ""),
        row(BARE_GROUP, ConvKind::Group, "  "),
        row(CHANNEL, ConvKind::Channel, ""),
        row(MEETING, ConvKind::Meeting, "Sprint review"),
    ] {
        store.upsert_conversation(&r).unwrap();
    }
    let h = Harness::with_store(store);
    // Tall enough that every row is on screen.
    h.app
        .window()
        .set_size(slint::LogicalSize::new(1200., 2400.));
    h.engine
        .test_set_members(GROUP, &[demo::ME, "8:orgid:m1", "8:orgid:m2"]);
    h.runner()(Cmd::SetFilter("all".into()));
    h
}

#[test]
fn a_group_chat_without_a_topic_lists_its_members_or_says_group_chat() {
    let h = harness();
    let t = titles(&h.app);
    assert_eq!(t[GROUP], "Ann, Bo");
    assert_eq!(t[BARE_GROUP], "Group chat");
}

#[test]
fn a_channel_without_a_name_is_general() {
    let h = harness();
    assert_eq!(titles(&h.app)[CHANNEL], "General");
}

/// How many elements carry the accessible `label`: the list's rows (the Meetings section may be
/// collapsed) and, when the open chat draws one, the header's avatar.
fn glyphs(h: &Harness, label: &str) -> usize {
    ElementHandle::find_by_accessible_label(&h.app, label).count()
}

#[test]
fn a_meeting_chats_header_shows_the_calendar_glyph_not_initials() {
    let h = harness();
    h.select(NOTES);
    let in_list = glyphs(&h, "Meeting chat");
    h.select(MEETING);
    assert_eq!(h.app.get_current_kind(), "meeting");
    assert_eq!(glyphs(&h, "Meeting chat"), in_list + 1);
    h.select(NOTES);
    assert_eq!(glyphs(&h, "Meeting chat"), in_list);
}

#[test]
fn a_group_chats_header_shows_the_people_glyph_when_it_has_no_picture() {
    let h = harness();
    h.select(NOTES);
    let in_list = glyphs(&h, "Group chat");
    h.select(GROUP);
    assert_eq!(h.app.get_current_kind(), "group");
    assert_eq!(glyphs(&h, "Group chat"), in_list + 1);
    assert!(!h.app.get_current_has_avatar());
}
