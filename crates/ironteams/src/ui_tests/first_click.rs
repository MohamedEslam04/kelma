//! The live test saw a first click do nothing after some interactions, a second one work. Clicks
//! here arrive as winit delivers them (`live_click`), with the pointer resting between them so
//! the hover timers (cards, tips) run.
//!
//! A menu or a clicked card whose outside press only closed it ate that click: the "…" menu at
//! the top right and a clicked profile card now close from the window's press report
//! (`pointer_pressed`) and the press reaches what is under it, like the message and profile
//! menus before them.

use super::*;
use crate::{PeopleStore, PersonItem, SettingsStore, TopBarStore};

/// A click as winit delivers it: the window hook sees the press first (`pointer_pressed`), then
/// the buffered move, the press and, `hold_ms` later, the release (a right release reaches the
/// hook first too).
fn live_click(app: &AppWindow, at: LogicalPosition, button: PointerEventButton, hold_ms: u64) {
    crate::pointer_pressed(app, at);
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    app.window().dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    testing::mock_elapsed_time(std::time::Duration::from_millis(hold_ms));
    if button == PointerEventButton::Right {
        crate::right_click_released(app);
    }
    app.window().dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    testing::mock_elapsed_time(std::time::Duration::from_millis(16));
}

fn center(e: &ElementHandle) -> LogicalPosition {
    let (l, t, r, b) = rect(e);
    LogicalPosition::new((l + r) / 2., (t + b) / 2.)
}

/// The pointer rests: time passes in frames, so the window's timers fire as they would.
fn rest(ms: u64) {
    for _ in 0..ms / 50 {
        testing::mock_elapsed_time(std::time::Duration::from_millis(50));
    }
}

/// A short tap, a usual click and one held past the lists' press delay (100 ms).
const HOLDS: [u64; 3] = [5, 50, 150];

fn more_button(app: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_element_type_name(app, "MoreButton")
        .next()
        .expect("the … button")
}

fn settings_row(app: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, "Settings")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button))
        .expect("the Settings row")
}

fn storyline_tab(app: &AppWindow) -> Option<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, "Storyline")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Tab))
}

#[test]
fn the_more_menus_settings_row_opens_settings_with_one_click() {
    for hold in HOLDS {
        let app = app_with_messages(vec![msg("m1", "theirs", false)]);
        live_click(
            &app,
            center(&more_button(&app)),
            PointerEventButton::Left,
            hold,
        );
        assert!(
            app.global::<TopBarStore>().get_more_open(),
            "{hold} ms: open"
        );
        rest(1500);
        live_click(
            &app,
            center(&settings_row(&app)),
            PointerEventButton::Left,
            hold,
        );
        assert!(
            app.global::<SettingsStore>().get_open(),
            "{hold} ms: Settings"
        );
        assert!(!app.global::<TopBarStore>().get_more_open());
    }
}

/// A click outside the "…" menu closes it and reaches the rail; the "…" button still toggles it.
#[test]
fn a_click_outside_the_more_menu_closes_it_and_reaches_the_rail() {
    let app = app_with_messages(vec![msg("m1", "theirs", false)]);
    let top = app.global::<TopBarStore>();
    live_click(
        &app,
        center(&more_button(&app)),
        PointerEventButton::Left,
        50,
    );
    assert!(top.get_more_open());
    // A press on the menu's own rows leaves it open.
    crate::pointer_pressed(&app, center(&settings_row(&app)));
    assert!(top.get_more_open(), "a press inside keeps it open");

    rest(1500);
    live_click(
        &app,
        center(&rail_entry(&app, 3)),
        PointerEventButton::Left,
        50,
    );
    assert!(!top.get_more_open(), "closed");
    assert_eq!(
        app.get_view(),
        "calendar",
        "the same click switched the view"
    );

    // The "…" button opens it and closes it again.
    live_click(
        &app,
        center(&more_button(&app)),
        PointerEventButton::Left,
        50,
    );
    assert!(top.get_more_open());
    live_click(
        &app,
        center(&more_button(&app)),
        PointerEventButton::Left,
        50,
    );
    assert!(!top.get_more_open(), "the button closed it");
    live_click(
        &app,
        center(&more_button(&app)),
        PointerEventButton::Left,
        50,
    );
    assert!(top.get_more_open(), "and opens it again");
}

/// A clicked profile card closes on a press outside it and the press reaches the rail; the
/// click on the photo of the person whose card it closed leaves it closed.
#[test]
fn a_click_outside_a_clicked_card_closes_it_and_reaches_the_rail() {
    let app = app_with_messages(vec![msg("m1", "theirs", false)]);
    let ps = app.global::<PeopleStore>();
    let opened = Log::default();
    {
        let l = opened.clone();
        ps.on_command(move |n, a| l.borrow_mut().push(format!("{n} {a}")));
    }
    let card = PersonItem {
        id: "8:orgid:00000000-0000-0000-0000-00000000000a".into(),
        name: "Sara Hassan".into(),
        ..Default::default()
    };
    ps.set_card(card.clone());
    ps.set_card_by_hover(false);
    ps.set_card_open(true);
    testing::mock_elapsed_time(std::time::Duration::from_millis(16));
    let shown = ElementHandle::find_by_element_type_name(&app, "ProfileCard")
        .next()
        .expect("the card");
    crate::pointer_pressed(&app, center(&shown));
    assert!(ps.get_card_open(), "a press inside keeps it open");

    rest(1500);
    live_click(
        &app,
        center(&rail_entry(&app, 3)),
        PointerEventButton::Left,
        50,
    );
    assert!(!ps.get_card_open(), "closed");
    assert_eq!(
        app.get_view(),
        "calendar",
        "the same click switched the view"
    );

    // The press that closed it on the person's photo: that click does not open it again.
    ps.set_card_open(true);
    testing::mock_elapsed_time(std::time::Duration::from_millis(16));
    crate::pointer_pressed(&app, LogicalPosition::new(4., 4.));
    ps.invoke_open_card(card.id.clone(), 0., 0., 32., 32., false);
    assert!(opened.borrow().is_empty(), "the photo toggled it closed");
    // The next click on it opens it.
    crate::pointer_pressed(&app, LogicalPosition::new(4., 4.));
    ps.invoke_open_card(card.id.clone(), 0., 0., 32., 32., false);
    assert_eq!(*opened.borrow(), vec![format!("profile {}", card.id)]);
}

/// The live case: a right click opens the message menu, one click on the rail closes it and
/// switches the view.
#[test]
fn after_a_right_click_menu_one_click_on_the_rail_switches_the_view() {
    for hold in HOLDS {
        let app = app_with_messages(vec![msg("m1", "some words from someone else", false)]);
        let bubble = bubble_of(&app, &rows(&app)[0]);
        let at = LogicalPosition::new(
            bubble.absolute_position().x + 40.,
            bubble.absolute_position().y + bubble.size().height - 18.,
        );
        live_click(&app, at, PointerEventButton::Right, hold);
        assert!(app.get_menu_open(), "{hold} ms: the menu opened");
        rest(1500);
        live_click(
            &app,
            center(&rail_entry(&app, 3)),
            PointerEventButton::Left,
            hold,
        );
        assert!(!app.get_menu_open(), "{hold} ms: closed");
        assert_eq!(app.get_view(), "calendar", "{hold} ms: switched");
    }
}

/// The live case: a channel opened from the Teams page, then one click on the rail's Chat.
#[test]
fn after_opening_a_channel_one_click_on_the_rail_switches_to_chat() {
    for hold in HOLDS {
        let h = Harness::new();
        {
            let run = h.runner();
            h.app.on_select_channel(move |id, title| {
                run(Cmd::SelectChannel(id.into(), title.into()))
            });
        }
        h.select(NOTES);
        h.app.invoke_navigate("teams".into());
        rest(200);
        let Some(ch) = ElementHandle::find_by_element_id(&h.app, "Sidebar::ct").last() else {
            panic!("a channel row: teams {}", h.app.get_teams().row_count());
        };
        live_click(&h.app, center(&ch), PointerEventButton::Left, hold);
        assert!(
            h.app.get_current_is_channel(),
            "{hold} ms: the channel opened"
        );
        rest(1500);
        live_click(
            &h.app,
            center(&rail_entry(&h.app, 1)),
            PointerEventButton::Left,
            hold,
        );
        assert_eq!(h.app.get_view(), "chat", "{hold} ms: switched");
    }
}

/// The live case: a chat opened from the chat list, then one click on its Storyline tab.
#[test]
fn after_opening_a_chat_one_click_on_storyline_switches_the_tab() {
    for hold in HOLDS {
        let h = Harness::new();
        h.app.invoke_navigate("chat".into());
        h.select(BACKEND);
        rest(200);
        let rows = ElementHandle::find_by_element_id(&h.app, "ChatItem::t").count();
        let mut done = false;
        for i in 0..rows {
            let row = ElementHandle::find_by_element_id(&h.app, "ChatItem::t")
                .nth(i)
                .unwrap();
            live_click(&h.app, center(&row), PointerEventButton::Left, hold);
            rest(1500);
            let Some(tab) = storyline_tab(&h.app) else {
                continue;
            };
            live_click(&h.app, center(&tab), PointerEventButton::Left, hold);
            assert_eq!(
                h.app.global::<crate::FindStore>().get_chat_tab(),
                "storyline",
                "{hold} ms: row {i}"
            );
            done = true;
            break;
        }
        assert!(done, "a chat with a Storyline tab among {rows} rows");
    }
}
