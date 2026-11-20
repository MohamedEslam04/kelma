//! The Activity view driven through Slint's testing backend. The backend's state reducer
//! (`ActivityState`) is wired to the window callbacks the way `main.rs` wires the engine, so
//! clicks, filters and read state are tested end to end without a network.

use super::*;
use crate::backend::{ActivityState, activity_row_data};
use crate::ui::activity::row;
use crate::{ActivityRow, demo};
use store::Store;

const NOW: i64 = 1_795_000_000_000;

type Shared = Rc<RefCell<ActivityState>>;

/// Rows shown by the window, in order, as (accessible label, accessible description).
fn shown(app: &AppWindow) -> Vec<(String, String)> {
    let mut v: Vec<ElementHandle> =
        ElementHandle::find_by_element_type_name(app, "ActivityItem").collect();
    v.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    v.iter()
        .map(|e| {
            (
                e.accessible_label().unwrap().to_string(),
                e.accessible_description().unwrap().to_string(),
            )
        })
        .collect()
}

fn item_el(app: &AppWindow, n: usize) -> ElementHandle {
    let mut v: Vec<ElementHandle> =
        ElementHandle::find_by_element_type_name(app, "ActivityItem").collect();
    v.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    v.remove(n)
}

fn by_id(app: &AppWindow, id: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_element_id(app, id).collect()
}

fn push(app: &AppWindow, state: &Shared) {
    let st = state.borrow();
    let rows: Vec<ActivityRow> = st
        .visible()
        .iter()
        .map(|i| row(activity_row_data(i, "In chat with you".into(), None)))
        .collect();
    app.set_activity_items(ModelRc::new(VecModel::from(rows)));
    app.set_activity_unread(st.unread() as i32);
}

/// A window on the Activity view over the demo feed, with the callbacks wired to the reducer.
fn feed() -> (AppWindow, Shared, Log) {
    let store = Store::open_in_memory().unwrap();
    demo::seed(&store, NOW).unwrap();
    let items = demo::activity(&store, NOW);
    testing::init_no_event_loop();
    let app = AppWindow::new().unwrap();
    app.window().set_size(LogicalSize::new(1200., 800.));
    app.set_login_state("ready".into());
    let state: Shared = Rc::default();
    state.borrow_mut().set_items(items);
    let log = Log::default();
    app.set_view("activity".into());
    push(&app, &state);
    app.show().unwrap();

    let wire = |f: &dyn Fn(&AppWindow, &Shared, &Log)| f(&app, &state, &log);
    wire(&|app, state, log| {
        let (w, st, l) = (app.as_weak(), state.clone(), log.clone());
        app.on_activity_select(move |id| {
            l.borrow_mut().push(format!("select {id}"));
            let w = w.upgrade().unwrap();
            let item = {
                let mut s = st.borrow_mut();
                s.selected = Some(id.to_string());
                s.set_read(&id, true);
                s.item(&id).cloned().unwrap()
            };
            push(&w, &st);
            // The backend sends a short window with the source a few rows from the top.
            let mut msgs: Vec<Msg> = (0..11)
                .map(|i| msg(&format!("m{i}"), "context", false))
                .collect();
            msgs[2] = msg(&item.source_message_id, "the source", false);
            w.set_activity_preview(ModelRc::new(VecModel::from(msgs)));
            w.set_activity_highlight(item.source_message_id.clone().into());
            w.set_activity_preview_id(id);
            w.set_activity_preview_is_channel(item.source_thread_id.ends_with("@thread.tacv2"));
        });
        let (w, st, l) = (app.as_weak(), state.clone(), log.clone());
        app.on_activity_set_filter(move |f| {
            l.borrow_mut().push(format!("filter {f}"));
            st.borrow_mut().set_filter(&f);
            push(&w.upgrade().unwrap(), &st);
        });
        let (w, st, l) = (app.as_weak(), state.clone(), log.clone());
        app.on_activity_search(move |q| {
            l.borrow_mut().push(format!("search {q}"));
            st.borrow_mut().set_search(q.to_string());
            push(&w.upgrade().unwrap(), &st);
        });
        let (w, st, l) = (app.as_weak(), state.clone(), log.clone());
        app.on_activity_mark(move |id, read| {
            l.borrow_mut().push(format!("mark {id} {read}"));
            st.borrow_mut().set_read(&id, read);
            push(&w.upgrade().unwrap(), &st);
        });
        let (w, st, l) = (app.as_weak(), state.clone(), log.clone());
        app.on_activity_mark_all(move || {
            l.borrow_mut().push("mark-all".into());
            st.borrow_mut().mark_all_read(NOW + 1);
            push(&w.upgrade().unwrap(), &st);
        });
        let l = log.clone();
        app.on_activity_go_to(move || l.borrow_mut().push("go-to".into()));
        let l = log.clone();
        app.on_activity_reply(move |t| l.borrow_mut().push(format!("reply {t}")));
        let l = log.clone();
        app.on_activity_settings(move || l.borrow_mut().push("settings".into()));
    });
    (app, state, log)
}

fn unread_labels(app: &AppWindow) -> usize {
    shown(app)
        .iter()
        .filter(|(l, _)| l.ends_with(", unread"))
        .count()
}

#[test]
fn list_shows_each_item_with_title_preview_and_location() {
    let (app, _state, _log) = feed();
    let rows = shown(&app);
    assert_eq!(rows.len(), 6);
    assert_eq!(rows[0].0, "Sara Hassan mentioned you, unread");
    assert_eq!(
        rows[0].1,
        "@Mohamed can you review the retry change? · In chat with you"
    );
    assert_eq!(rows[1].0, "Omar Khaled reacted to your message, unread");
    assert_eq!(rows[2].0, "Youssef Samir posted a new message, unread");
    assert_eq!(rows[3].0, "Ahmed Ali replied to your message");
    assert_eq!(unread_labels(&app), 3, "read rows carry no unread mark");
}

#[test]
fn filters_narrow_the_list_and_toggle_off() {
    let (app, _state, log) = feed();
    let click = |id: &str| by_id(&app, id)[0].mock_single_click(PointerEventButton::Left);

    click("ActivityView::f-unread");
    assert_eq!(shown(&app).len(), 3, "unread only");
    assert_eq!(unread_labels(&app), 3);
    click("ActivityView::f-unread");
    assert_eq!(shown(&app).len(), 6, "second click turns the filter off");

    click("ActivityView::f-mentions");
    let titles: Vec<String> = shown(&app).into_iter().map(|r| r.0).collect();
    assert_eq!(
        titles,
        [
            "Sara Hassan mentioned you, unread",
            "Mona Adel mentioned a tag with you",
            "Omar Khaled mentioned Everyone"
        ]
    );
    click("ActivityView::f-tags");
    assert_eq!(shown(&app).len(), 1, "tag mentions");
    assert_eq!(shown(&app)[0].0, "Mona Adel mentioned a tag with you");
    assert_eq!(app.get_activity_filter(), "tags");
    assert_eq!(
        *log.borrow(),
        [
            "filter unread",
            "filter all",
            "filter mentions",
            "filter tags"
        ]
    );
}

#[test]
fn filter_shortcuts_ctrl_alt_u_and_m() {
    let (app, _state, _log) = feed();
    let chord = |ch: char| {
        key(&app, Key::Control, true);
        key(&app, Key::Alt, true);
        key(&app, ch, true);
        key(&app, ch, false);
        key(&app, Key::Alt, false);
        key(&app, Key::Control, false);
    };
    chord('u');
    assert_eq!(app.get_activity_filter(), "unread");
    assert_eq!(shown(&app).len(), 3);
    chord('u');
    assert_eq!(app.get_activity_filter(), "all");
    chord('m');
    assert_eq!(app.get_activity_filter(), "mentions");
    assert_eq!(shown(&app).len(), 3);
}

#[test]
fn search_filters_by_actor_preview_and_topic() {
    let (app, _state, log) = feed();
    by_id(&app, "ActivityView::search-input")[0].mock_single_click(PointerEventButton::Left);
    type_text(&app, "redis");
    assert_eq!(log.borrow().last().unwrap(), "search redis");
    let rows = shown(&app);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "Omar Khaled mentioned Everyone");
}

#[test]
fn badge_counts_activity_plus_chats_in_rail_and_title() {
    let app = app_with_messages(vec![]);
    assert_eq!(app.get_window_title(), "kelma");
    app.set_activity_unread(3);
    app.set_chats_unread(2);
    assert_eq!(app.get_badge_total(), 5);
    assert_eq!(app.get_window_title(), "(5) kelma");

    let badge = |label: &str| ElementHandle::find_by_accessible_label(&app, label).count();
    assert_eq!(badge("3 unread"), 1, "Activity rail badge");
    assert_eq!(badge("2 unread"), 1, "Chat rail badge");
    app.set_activity_unread(0);
    assert_eq!(badge("3 unread"), 0, "no badge at zero");
    assert_eq!(app.get_window_title(), "(2) kelma");
    app.set_chats_unread(0);
    assert_eq!(app.get_window_title(), "kelma");
}

#[test]
fn clicking_an_item_marks_it_read_and_opens_the_preview_on_the_source() {
    let (app, state, log) = feed();
    assert_eq!(app.get_activity_unread(), 3);
    assert_eq!(by_id(&app, "ActivityView::go").len(), 0, "no preview yet");

    item_el(&app, 0).mock_single_click(PointerEventButton::Left);
    assert!(log.borrow()[0].starts_with("select "));
    assert_eq!(app.get_activity_unread(), 2, "badge drops by one");
    assert_eq!(unread_labels(&app), 2);
    assert_eq!(
        shown(&app)[0].0,
        "Sara Hassan mentioned you",
        "no longer bold"
    );
    assert!(state.borrow().selected.is_some());

    // The preview exists, offers "Go to chat" and no reply box for a chat message.
    assert_eq!(by_id(&app, "ActivityView::go").len(), 1);
    assert_eq!(by_id(&app, "ActivityView::reply-input").len(), 0);

    // The highlighted source row is inside the pane, near the top.
    let pane = &by_id(&app, "PreviewList::pv")[0];
    let (top, bottom) = (
        pane.absolute_position().y,
        pane.absolute_position().y + pane.size().height,
    );
    let marker = by_id(&app, "PreviewList::hl-marker");
    assert_eq!(marker.len(), 1, "exactly the source row is highlighted");
    let y = marker[0].absolute_position().y;
    assert!(
        y >= top && y + marker[0].size().height <= bottom + 1.,
        "source row at {y} is outside the pane {top}..{bottom}"
    );
}

#[test]
fn channel_posts_get_a_reply_box_and_go_to_channel() {
    let (app, _state, log) = feed();
    // The third item is a channel post.
    item_el(&app, 2).mock_single_click(PointerEventButton::Left);
    let go = &by_id(&app, "ActivityView::go")[0];
    let reply = &by_id(&app, "ActivityView::reply-input")[0];

    reply.mock_single_click(PointerEventButton::Left);
    type_text(&app, "thanks!");
    enter(&app);
    assert_eq!(log.borrow().last().unwrap(), "reply thanks!");

    go.mock_single_click(PointerEventButton::Left);
    assert_eq!(log.borrow().last().unwrap(), "go-to");
    assert_eq!(app.get_view(), "chat", "Go to channel switches to Chat");
}

#[test]
fn mark_all_read_by_shortcut_and_by_menu() {
    let (app, _state, log) = feed();
    key(&app, Key::Shift, true);
    key(&app, Key::Escape, true);
    key(&app, Key::Escape, false);
    key(&app, Key::Shift, false);
    assert_eq!(log.borrow().last().unwrap(), "mark-all");
    assert_eq!(unread_labels(&app), 0);
    assert_eq!(app.get_activity_unread(), 0);

    // Make something unread again, then use the "…" menu.
    item_el(&app, 1).mock_single_click(PointerEventButton::Right);
    by_id(&app, "ActivityItem::toggle")[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(unread_labels(&app), 1, "context menu marks one item unread");

    by_id(&app, "ActivityView::more")[0].mock_single_click(PointerEventButton::Left);
    by_id(&app, "ActivityView::mark-all-entry")[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(unread_labels(&app), 0);
    assert_eq!(log.borrow().iter().filter(|l| *l == "mark-all").count(), 2);
}

#[test]
fn context_menu_toggles_read_state_per_item() {
    let (app, _state, log) = feed();
    // Item 0 is unread: the menu offers "Mark as read".
    item_el(&app, 0).mock_single_click(PointerEventButton::Right);
    by_id(&app, "ActivityItem::toggle")[0].mock_single_click(PointerEventButton::Left);
    assert!(log.borrow().last().unwrap().ends_with("true"));
    assert_eq!(unread_labels(&app), 2);
    // Item 3 is read: the menu offers "Mark as unread".
    item_el(&app, 3).mock_single_click(PointerEventButton::Right);
    by_id(&app, "ActivityItem::toggle")[0].mock_single_click(PointerEventButton::Left);
    assert!(log.borrow().last().unwrap().ends_with("false"));
    assert_eq!(unread_labels(&app), 3);
    assert!(
        log.borrow().iter().all(|l| !l.starts_with("select")),
        "right click does not open the preview"
    );
}

#[test]
fn notification_settings_entry_is_in_the_menu() {
    let (app, _state, log) = feed();
    by_id(&app, "ActivityView::more")[0].mock_single_click(PointerEventButton::Left);
    by_id(&app, "ActivityView::settings-entry")[0].mock_single_click(PointerEventButton::Left);
    assert_eq!(*log.borrow(), ["settings"]);
}

#[test]
fn long_titles_previews_and_locations_stay_inside_the_feed_rows() {
    use super::overflow::{LONG, LONG_AR, LONG_WORD, assert_texts_inside};
    let (app, _state, _log) = feed();
    let item = |id: &str, text: &str, rtl: bool| ActivityRow {
        id: id.into(),
        title: text.into(),
        initials: "QP".into(),
        badge: "@".into(),
        unread: true,
        preview: text.into(),
        time: "Wednesday 12:30 PM".into(),
        location: format!("{text} › {text}").into(),
        rtl_preview: rtl,
        ..Default::default()
    };
    app.set_activity_items(ModelRc::new(VecModel::from(vec![
        item("a", LONG, false),
        item("b", LONG_AR, true),
        item("c", LONG_WORD, false),
    ])));
    let rows: Vec<_> = ElementHandle::find_by_element_type_name(&app, "ActivityItem").collect();
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert_texts_inside(row, "activity row");
    }
}

#[test]
fn preview_rows_take_reactions_and_open_the_message_menu() {
    let (app, _state, _log) = feed();
    let log = record_actions(&app);
    item_el(&app, 0).mock_single_click(PointerEventButton::Left);
    let pane = &by_id(&app, "PreviewList::pv")[0];
    let top = pane.absolute_position().y;
    let mut preview: Vec<ElementHandle> = rows(&app)
        .into_iter()
        .filter(|r| r.absolute_position().y >= top)
        .collect();
    preview.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    let row = &preview[0];

    hover(&app, row);
    let reacts = quick_reactions(&app);
    assert_eq!(reacts.len(), 4, "the hover bar shows in the preview");
    click(&reacts[0]);
    assert_eq!(log.borrow().last().unwrap(), "react m0 like");

    open_menu(&app, row);
    assert!(app.get_menu_preview());
    let labels = menu_labels(&app);
    for gone in ["Reply with quote", "Mark as unread", "Edit", "Delete"] {
        assert!(
            !labels.contains(&gone.to_string()),
            "{gone:?} in {labels:?}"
        );
    }
    click(&menu_item(&app, "Copy message link"));
    assert_eq!(log.borrow().last().unwrap(), "copy-link m0");
}
