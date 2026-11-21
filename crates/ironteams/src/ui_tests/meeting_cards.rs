//! Meeting events in a meeting chat, through the real engine and window: a call's start and end
//! are one card, a recording's status updates are one card in its final state, the transcript is
//! a small row, and the chat list reads "Meeting ended · 1h 3m" instead of "Play".

use super::*;
use crate::RecapsStore;
use crate::backend::meeting_cards::tests::{CHAT, ended, meeting, recording};
use serde_json::json;
use store::{ConvKind, ConversationRow};

fn harness() -> Harness {
    let h = Harness::new();
    h.engine.test_store(|st| {
        st.upsert_conversation(&ConversationRow {
            id: CHAT.into(),
            kind: ConvKind::Meeting,
            title: "Weekly sync".into(),
            team_id: None,
            team_name: None,
            last_activity_ms: teams_proto::time::now_ms(),
            last_preview: String::new(),
            unread: false,
            favorite: false,
            read_horizon_ms: 0,
        })
        .unwrap();
        for r in meeting() {
            st.upsert_message(&r).unwrap();
        }
    });
    h.select(CHAT);
    h
}

fn cards(app: &AppWindow) -> Vec<(String, crate::MeetingCard)> {
    app.get_messages()
        .iter()
        .filter_map(|m| Some((m.id.to_string(), m.meeting.row_data(0)?)))
        .collect()
}

fn press(app: &AppWindow, label: &str) {
    find_label(app, label)
        .into_iter()
        .find(|e| e.accessible_role() == Some(testing::AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no button labelled {label:?}"))
        .invoke_accessible_default_action();
}

#[test]
fn a_meeting_shows_one_call_card_and_one_recording_card_in_its_final_state() {
    let h = harness();
    let ids: Vec<String> = h
        .app
        .get_messages()
        .iter()
        .map(|m| m.id.to_string())
        .collect();
    assert_eq!(
        ids,
        ["c1", "t1", "h1", "r5"],
        "the superseded events are left out"
    );

    let cards = cards(&h.app);
    let calls: Vec<_> = cards.iter().filter(|(_, c)| c.kind == "call").collect();
    let recordings: Vec<_> = cards
        .iter()
        .filter(|(_, c)| c.kind == "recording")
        .collect();
    assert_eq!(calls.len(), 1, "one call card");
    assert_eq!(recordings.len(), 1, "one recording card");

    let (_, call) = calls[0];
    assert_eq!(call.title, "Meeting ended · 1h 3m 12s");
    assert_eq!(call.people.row_count(), 5);
    assert_eq!(call.more, 1);
    let (_, rec) = recordings[0];
    assert_eq!(rec.state, "ready");
    assert_eq!(rec.detail, "58m 35s · Recorded by Grace Hopper");

    // Drawn as cards, not grey lines.
    assert!(!find_label(&h.app, "Meeting ended · 1h 3m 12s").is_empty());
    assert!(!find_label(&h.app, "Transcript available").is_empty());

    // Play opens the recording in the browser.
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let o = opened.clone();
    h.app
        .on_open_url(move |u| o.borrow_mut().push(u.to_string()));
    press(&h.app, "Play Weekly sync-20261110_1230-Meeting Recording");
    assert_eq!(
        *opened.borrow(),
        ["https://contoso-my.sharepoint.invalid/:v:/g/rec"]
    );

    // View recap switches to the chat's Recap tab.
    let asked = Rc::new(RefCell::new(Vec::<String>::new()));
    let a = asked.clone();
    h.app
        .global::<RecapsStore>()
        .on_command(move |n, arg| a.borrow_mut().push(format!("{n} {arg}")));
    press(&h.app, "View recap");
    assert_eq!(*asked.borrow(), [format!("recaps-chat {CHAT}")]);
    assert_eq!(h.app.global::<crate::FindStore>().get_chat_tab(), "recap");
}

#[test]
fn the_chat_list_reads_what_happened() {
    let message = |id: &str, message_type: &str, content: &str| {
        teams_proto::model::message_from_json(
            &json!({"id": id, "conversationid": CHAT, "from": "8:orgid:aaaa-1",
                "composetime": "2026-11-10T11:33:46Z", "messagetype": message_type,
                "content": content}),
            CHAT,
        )
        .unwrap()
    };
    let me = "8:orgid:aaaa-1";
    let preview = |m: teams_proto::model::Message| crate::backend::util::preview_of(&m, me);
    assert_eq!(
        preview(message("c2", "Event/Call", &ended())),
        "Meeting ended · 1h 3m"
    );
    assert_eq!(
        preview(message(
            "r5",
            "RichText/Media_CallRecording",
            &recording("Success", "Recording+Transcript")
        )),
        "Recording: Weekly sync-20261110_1230-Meeting Recording"
    );
    assert_eq!(
        preview(message(
            "r1",
            "RichText/Media_CallRecording",
            &recording("Initial", "Recording")
        )),
        "Recording…"
    );
}
