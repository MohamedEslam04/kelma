//! The Saved view (plan chat 9.8) driven through the real window: the chat list's "…" menu opens
//! it, it lists the locally saved messages newest first, a click opens the chat and lands on the
//! message, and "Unsave" takes a row out of the list and the store. `FindStore` commands are
//! wired to the engine the way `main.rs` wires them; there is no network.

use super::*;
use crate::FindStore;
use crate::backend::FindCmd;
use store::{MessageRow, SavedMessage};

fn wire(h: &Harness) {
    let run = h.runner();
    h.app.global::<FindStore>().on_command(move |name, arg| {
        if let Some(c) = FindCmd::parse(&name, &arg) {
            run(Cmd::Find(c));
        }
    });
}

fn fs(app: &AppWindow) -> FindStore<'_> {
    app.global::<FindStore>()
}

fn click(el: &ElementHandle) {
    el.mock_single_click(PointerEventButton::Left);
}

/// The message of the demo chat whose text holds `needle`.
fn message_with(h: &Harness, needle: &str) -> MessageRow {
    h.engine
        .test_store(|st| st.messages_before(BACKEND, None, 100).unwrap())
        .into_iter()
        .find(|m| m.content_html.contains(needle))
        .unwrap_or_else(|| panic!("no demo message with {needle:?}"))
}

fn save(h: &Harness, m: &MessageRow, at: i64) {
    h.engine.test_store(|st| {
        st.save_message(&SavedMessage {
            conversation_id: m.conversation_id.clone(),
            message_id: m.id.clone(),
            root_id: m.id.clone(),
            sender_name: m.sender_name.clone(),
            preview: "saved preview".into(),
            saved_ms: at,
        })
        .unwrap();
    });
}

fn hit_items(app: &AppWindow) -> Vec<ElementHandle> {
    let mut v: Vec<_> = ElementHandle::find_by_element_type_name(app, "HitItem").collect();
    v.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    v
}

#[test]
fn saved_lists_the_saved_messages_newest_first() {
    let h = Harness::new();
    wire(&h);
    let older = message_with(&h, "it was deployed to");
    let newer = message_with(&h, "refresh logic");
    save(&h, &older, 1_000);
    save(&h, &newer, 2_000);
    h.select(NOTES);

    h.runner()(Cmd::Find(FindCmd::Quick("saved".into())));
    assert!(fs(&h.app).get_quick_open());
    assert_eq!(fs(&h.app).get_quick_kind(), "saved");
    let rows: Vec<_> = fs(&h.app).get_quick_rows().iter().collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].id.contains(&newer.id), "newest saved on top");
    assert!(rows[1].id.contains(&older.id));
    assert_eq!(rows[0].subtitle, "Backend Team");
    assert!(rows[0].snippet_plain.contains("saved preview"));
    assert!(!find_label(&h.app, "Saved").is_empty(), "the view's title");
    assert!(!find_label(&h.app, "Unsave").is_empty());
}

#[test]
fn an_empty_saved_view_says_so() {
    let h = Harness::new();
    wire(&h);
    h.select(NOTES);
    h.runner()(Cmd::Find(FindCmd::Quick("saved".into())));
    assert_eq!(fs(&h.app).get_quick_rows().row_count(), 0);
    assert!(fs(&h.app).get_quick_open());
}

#[test]
fn a_click_on_a_saved_message_opens_its_chat_and_lands_on_it() {
    let h = Harness::new();
    wire(&h);
    let m = message_with(&h, "it was deployed to");
    save(&h, &m, 1_000);
    h.select(NOTES);
    h.runner()(Cmd::Find(FindCmd::Quick("saved".into())));

    click(&hit_items(&h.app)[0]);
    assert_eq!(h.app.get_current_id(), BACKEND);
    assert!(
        !fs(&h.app).get_quick_open(),
        "opening a chat closes the view"
    );
    let flashed: Vec<_> = h
        .app
        .get_messages()
        .iter()
        .filter(|x| x.flash)
        .map(|x| x.plain.to_string())
        .collect();
    assert_eq!(flashed.len(), 1, "the saved message is highlighted");
    assert!(flashed[0].contains("deployed to staging"), "{flashed:?}");
}

#[test]
fn unsave_removes_a_row_from_the_list_and_the_store() {
    let h = Harness::new();
    wire(&h);
    let a = message_with(&h, "it was deployed to");
    let b = message_with(&h, "refresh logic");
    save(&h, &a, 1_000);
    save(&h, &b, 2_000);
    h.select(NOTES);
    h.runner()(Cmd::Find(FindCmd::Quick("saved".into())));

    // The newest row is first: unsave it.
    click(&find_label(&h.app, "Unsave")[0]);
    let rows: Vec<_> = fs(&h.app).get_quick_rows().iter().collect();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].id.contains(&a.id));
    let left = h.engine.test_store(|st| st.saved_messages().unwrap());
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].message_id, a.id);
}
