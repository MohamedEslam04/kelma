//! UI-thread side of the Calls page: fills the `CallsPageStore` global and sends its commands to
//! the engine (backend/calls_page.rs).

use super::{UiHandle, img, model, rgb};
use crate::backend::calls_page::{HistoryRow, PersonRow, VoicemailRow};
use crate::backend::{CallsCmd, CallsData, Cmd};
use crate::{AppWindow, CallHistoryItem, CallPersonItem, CallsPageStore, VoicemailItem};
use slint::ComponentHandle;

/// Shows the Calls item on the rail and sends the page's commands to the engine.
pub fn install(w: &AppWindow, tx: tokio::sync::mpsc::UnboundedSender<Cmd>) {
    let s = w.global::<CallsPageStore>();
    s.set_available(true);
    s.on_command(move |name, arg| {
        if let Some(c) = CallsCmd::parse(&name, &arg) {
            let _ = tx.send(Cmd::CallsPage(c));
        }
    });
}

fn history_item(r: HistoryRow) -> CallHistoryItem {
    let (avatar, has_avatar) = img(&r.avatar);
    CallHistoryItem {
        id: r.id.into(),
        mri: r.mri.into(),
        name: r.name.into(),
        initials: r.initials.into(),
        color: rgb(r.color),
        avatar,
        has_avatar,
        direction: r.direction.into(),
        outcome: r.outcome.into(),
        missed: r.missed,
        when: r.when.into(),
        duration: r.duration.into(),
        group: r.group,
        speed: r.speed,
        number: r.number.into(),
    }
}

fn person_item(p: PersonRow) -> CallPersonItem {
    let (avatar, has_avatar) = img(&p.avatar);
    CallPersonItem {
        mri: p.mri.into(),
        name: p.name.into(),
        detail: p.detail.into(),
        initials: p.initials.into(),
        color: rgb(p.color),
        avatar,
        has_avatar,
        speed: p.speed,
    }
}

fn voicemail_item(v: VoicemailRow) -> VoicemailItem {
    VoicemailItem {
        id: v.id.into(),
        name: v.name.into(),
        initials: v.initials.into(),
        color: rgb(v.color),
        when: v.when.into(),
        duration: v.duration.into(),
        transcript: v.transcript.into(),
        unread: v.unread,
    }
}

fn set(w: &AppWindow, d: CallsData) {
    let s = w.global::<CallsPageStore>();
    s.set_filter(d.filter.into());
    s.set_history(model(d.history.into_iter().map(history_item).collect()));
    s.set_voicemails(model(
        d.voicemails.into_iter().map(voicemail_item).collect(),
    ));
    s.set_results(model(d.results.into_iter().map(person_item).collect()));
    s.set_speed(model(d.speed.into_iter().map(person_item).collect()));
    s.set_dial(d.dial.into());
    s.set_loading(d.loading);
    s.set_error(d.error.into());
    s.set_voicemail_error(d.voicemail_error.into());
}

impl UiHandle {
    pub fn calls_page(&self, d: CallsData) {
        self.run(move |w| set(&w, d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use i_slint_backend_testing::{self as testing, ElementHandle};
    use slint::LogicalSize;
    use slint::platform::PointerEventButton;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn app() -> AppWindow {
        thread_local! { static INIT: std::sync::Once = const { std::sync::Once::new() }; }
        INIT.with(|once| once.call_once(testing::init_no_event_loop));
        let app = AppWindow::new().unwrap();
        app.window().set_size(LogicalSize::new(1400., 900.));
        app.set_login_state("ready".into());
        app
    }

    fn commands(app: &AppWindow) -> Rc<RefCell<Vec<(String, String)>>> {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        app.global::<CallsPageStore>()
            .on_command(move |n, a| l.borrow_mut().push((n.to_string(), a.to_string())));
        log
    }

    fn row(id: &str, mri: &str, missed: bool) -> HistoryRow {
        HistoryRow {
            id: id.into(),
            mri: mri.into(),
            name: format!("Person {id}"),
            initials: "P".into(),
            color: 0x336699,
            avatar: None,
            direction: "incoming".into(),
            outcome: if missed { "missed" } else { "accepted" }.into(),
            missed,
            when: "10:00".into(),
            duration: if missed { "" } else { "1m 5s" }.into(),
            group: mri.is_empty(),
            speed: false,
            number: String::new(),
        }
    }

    fn click(app: &AppWindow, id: &str, nth: usize) {
        ElementHandle::find_by_element_id(app, id)
            .nth(nth)
            .unwrap_or_else(|| panic!("no {id} #{nth}"))
            .mock_single_click(PointerEventButton::Left);
    }

    #[test]
    fn history_rows_call_back_and_group_calls_have_no_call_button() {
        let app = app();
        app.set_view("calls".into());
        let log = commands(&app);
        set(
            &app,
            CallsData {
                filter: "all".into(),
                history: vec![row("a", "8:orgid:a", true), row("g", "", false)],
                ..Default::default()
            },
        );
        assert_eq!(
            ElementHandle::find_by_element_id(&app, "HistoryLine::call-back").count(),
            1,
            "only the 1:1 call has Call back"
        );
        click(&app, "HistoryLine::call-back", 0);
        assert_eq!(
            *log.borrow(),
            [("call".to_string(), "8:orgid:a".to_string())]
        );
    }

    #[test]
    fn the_voicemail_filter_lists_voicemail_with_play() {
        let app = app();
        app.set_view("calls".into());
        let log = commands(&app);
        set(
            &app,
            CallsData {
                filter: "voicemail".into(),
                voicemails: vec![VoicemailRow {
                    id: "vm1".into(),
                    mri: String::new(),
                    name: "Caller".into(),
                    initials: "C".into(),
                    color: 0x123456,
                    when: "Yesterday".into(),
                    duration: "12s".into(),
                    transcript: "Please call back".into(),
                    unread: true,
                }],
                ..Default::default()
            },
        );
        click(&app, "VoicemailLine::play-button", 0);
        assert_eq!(
            *log.borrow(),
            [("voicemail-play".to_string(), "vm1".to_string())]
        );
    }

    #[test]
    fn a_long_voicemail_transcript_stays_on_one_line_inside_its_row() {
        let app = app();
        app.set_view("calls".into());
        let text = "Hello, this is a long message about the plans for next week and the \
            things we still need to settle before the end of the month, call me back.";
        let vm = |id: &str, name: &str| VoicemailRow {
            id: id.into(),
            mri: String::new(),
            name: name.into(),
            initials: "C".into(),
            color: 0x123456,
            when: "Yesterday".into(),
            duration: "12s".into(),
            transcript: text.into(),
            unread: false,
        };
        set(
            &app,
            CallsData {
                filter: "voicemail".into(),
                voicemails: vec![vm("a", "First Caller"), vm("b", "Second Caller")],
                ..Default::default()
            },
        );
        let preview = ElementHandle::find_by_accessible_label(&app, text)
            .next()
            .expect("the transcript is shown");
        assert!(
            preview.size().height < 20.,
            "one line: {:?}",
            preview.size()
        );
        let first = ElementHandle::find_by_accessible_label(&app, "First Caller")
            .next()
            .unwrap();
        let second = ElementHandle::find_by_accessible_label(&app, "Second Caller")
            .next()
            .unwrap();
        let row = second.absolute_position().y - first.absolute_position().y;
        assert!(
            preview.absolute_position().y + preview.size().height
                <= first.absolute_position().y + row,
            "the first row's text ends inside the first row"
        );
    }

    #[test]
    fn speed_dial_people_can_be_called() {
        let app = app();
        app.set_view("calls".into());
        let log = commands(&app);
        set(
            &app,
            CallsData {
                filter: "all".into(),
                speed: vec![PersonRow {
                    mri: "8:orgid:s".into(),
                    name: "Speedy".into(),
                    detail: String::new(),
                    initials: "S".into(),
                    color: 0x654321,
                    avatar: None,
                    speed: true,
                }],
                ..Default::default()
            },
        );
        click(&app, "PersonLine::call-button", 0);
        assert_eq!(
            *log.borrow(),
            [("call".to_string(), "8:orgid:s".to_string())]
        );
    }

    fn labelled(app: &AppWindow, label: &str) -> Vec<ElementHandle> {
        testing::mock_elapsed_time(std::time::Duration::from_millis(16));
        // Buttons only: a menu item's text carries the same label as the item.
        ElementHandle::find_by_accessible_label(app, label)
            .filter(|e| {
                e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Button)
            })
            .collect()
    }

    fn press(app: &AppWindow, label: &str) {
        labelled(app, label)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no element labelled {label:?}"))
            .mock_single_click(PointerEventButton::Left);
    }

    #[test]
    fn a_history_row_menu_calls_chats_copies_and_removes_from_view() {
        let app = app();
        app.set_view("calls".into());
        let log = commands(&app);
        let mut pstn = row("p", "4:+15550100000", false);
        pstn.name = "+1 555 010 0000".into();
        pstn.number = "+15550100000".into();
        set(
            &app,
            CallsData {
                filter: "all".into(),
                history: vec![row("a", "8:orgid:a", false), pstn, row("g", "", false)],
                ..Default::default()
            },
        );
        // A person: call back, chat, speed dial, copy the name, remove. No number to copy.
        press(&app, "More actions for Person a");
        // (Call back and Chat are also buttons of the rows: two rows with a person, plus the menu.)
        assert_eq!(labelled(&app, "Call back").len(), 3);
        assert_eq!(labelled(&app, "Chat").len(), 3);
        for item in ["Copy name", "Remove from view"] {
            assert_eq!(labelled(&app, item).len(), 1, "{item}");
        }
        assert_eq!(labelled(&app, "Add to speed dial").len(), 3);
        assert!(labelled(&app, "Copy number").is_empty());
        press(&app, "Copy name");
        assert_eq!(
            log.borrow().last().unwrap(),
            &("copy".to_string(), "Person a".to_string())
        );
        // The menu closed; a phone number can be copied too.
        press(&app, "More actions for +1 555 010 0000");
        press(&app, "Copy number");
        assert_eq!(
            log.borrow().last().unwrap(),
            &("copy".to_string(), "+15550100000".to_string())
        );
        press(&app, "More actions for +1 555 010 0000");
        press(&app, "Remove from view");
        assert_eq!(
            log.borrow().last().unwrap(),
            &("hide".to_string(), "p".to_string())
        );
        // A group call has no one to call or chat with, only the copy and the removal.
        press(&app, "More actions for Person g");
        // (The two rows with a person already have a Call back and a Chat button of their own.)
        assert_eq!(labelled(&app, "Call back").len(), 2);
        assert_eq!(labelled(&app, "Chat").len(), 2);
        assert!(!labelled(&app, "Remove from view").is_empty());
    }

    #[test]
    fn a_voicemail_can_be_called_back_marked_either_way_and_deleted() {
        let app = app();
        app.set_view("calls".into());
        let log = commands(&app);
        let vm = |id: &str, unread: bool| VoicemailRow {
            id: id.into(),
            mri: String::new(),
            name: format!("Caller {id}"),
            initials: "C".into(),
            color: 0x123456,
            when: "Yesterday".into(),
            duration: "12s".into(),
            transcript: "Call me".into(),
            unread,
        };
        set(
            &app,
            CallsData {
                filter: "voicemail".into(),
                voicemails: vec![vm("u", true), vm("r", false)],
                ..Default::default()
            },
        );
        press(&app, "Mark as read");
        assert_eq!(
            log.borrow().last().unwrap(),
            &("voicemail-read".to_string(), "u".to_string())
        );
        press(&app, "Mark as unread");
        assert_eq!(
            log.borrow().last().unwrap(),
            &("voicemail-unread".to_string(), "r".to_string())
        );
        press(&app, "Delete voicemail from Caller r");
        assert_eq!(
            log.borrow().last().unwrap(),
            &("voicemail-delete".to_string(), "r".to_string())
        );
        click(&app, "VoicemailLine::play-button", 0);
        assert_eq!(
            log.borrow().last().unwrap(),
            &("voicemail-play".to_string(), "u".to_string())
        );
        // Call back is on every row (the first Call back button is the first row's).
        labelled(&app, "Call back")[0].mock_single_click(PointerEventButton::Left);
        assert_eq!(
            log.borrow().last().unwrap(),
            &("voicemail-call".to_string(), "u".to_string())
        );
    }

    #[test]
    fn a_phone_number_in_the_search_box_shows_a_disabled_call_row() {
        let app = app();
        app.set_view("calls".into());
        let log = commands(&app);
        assert!(labelled(&app, "Call +15550100000").is_empty());
        set(
            &app,
            CallsData {
                filter: "all".into(),
                dial: "+15550100000".into(),
                ..Default::default()
            },
        );
        let row = labelled(&app, "Call +15550100000");
        assert_eq!(row.len(), 1, "a Call <number> row appears");
        assert_eq!(
            row[0].accessible_enabled(),
            Some(false),
            "kelma cannot dial phone numbers yet"
        );
        assert_eq!(
            row[0].accessible_description().unwrap_or_default().as_str(),
            "kelma can't call phone numbers yet."
        );
        row[0].invoke_accessible_default_action();
        assert!(log.borrow().is_empty(), "no call is placed");
    }

    #[test]
    fn long_names_and_transcripts_stay_inside_the_rows_of_the_calls_page() {
        use crate::ui_tests::overflow::{LONG, LONG_AR, assert_texts_inside};
        for width in [900., 1400.] {
            let app = app();
            app.window().set_size(LogicalSize::new(width, 700.));
            app.set_view("calls".into());
            let history = |id: &str, name: &str| HistoryRow {
                name: name.into(),
                outcome: "missed".into(),
                missed: true,
                when: "Wednesday, 12:30 PM".into(),
                duration: "1h 5m 20s".into(),
                ..row(id, "8:orgid:a", true)
            };
            let person = |name: &str| PersonRow {
                mri: "8:orgid:p".into(),
                name: name.into(),
                detail: LONG.into(),
                initials: "S".into(),
                color: 0x654321,
                avatar: None,
                speed: true,
            };
            let voicemail = |id: &str, name: &str| VoicemailRow {
                id: id.into(),
                mri: String::new(),
                name: name.into(),
                initials: "C".into(),
                color: 0x123456,
                when: "Wednesday, 12:30 PM".into(),
                duration: "12s".into(),
                transcript: LONG_AR.into(),
                unread: true,
            };
            set(
                &app,
                CallsData {
                    filter: "all".into(),
                    history: vec![history("a", LONG), history("b", LONG_AR)],
                    speed: vec![person(LONG), person(LONG_AR)],
                    results: vec![person(LONG)],
                    voicemails: vec![voicemail("v", LONG), voicemail("w", LONG_AR)],
                    ..Default::default()
                },
            );
            for kind in ["HistoryLine", "PersonLine"] {
                let rows: Vec<_> = ElementHandle::find_by_element_type_name(&app, kind).collect();
                assert!(!rows.is_empty(), "{kind} rows");
                for r in &rows {
                    assert_texts_inside(r, &format!("{kind}, window {width}px wide"));
                }
            }
            set(
                &app,
                CallsData {
                    filter: "voicemail".into(),
                    voicemails: vec![voicemail("v", LONG), voicemail("w", LONG_AR)],
                    ..Default::default()
                },
            );
            let rows: Vec<_> =
                ElementHandle::find_by_element_type_name(&app, "VoicemailLine").collect();
            assert_eq!(rows.len(), 2);
            for r in &rows {
                assert_texts_inside(r, &format!("VoicemailLine, window {width}px wide"));
            }
        }
    }

    #[test]
    fn a_long_calls_error_stays_inside_the_page() {
        use crate::ui_tests::overflow::{LONG, assert_texts_inside};
        let app = app();
        app.window().set_size(LogicalSize::new(900., 700.));
        app.set_view("calls".into());
        set(
            &app,
            CallsData {
                filter: "all".into(),
                error: format!("Could not load your call history: {LONG} {LONG}"),
                ..Default::default()
            },
        );
        let page = ElementHandle::find_by_element_type_name(&app, "CallsPage")
            .next()
            .expect("the calls page");
        assert_texts_inside(&page, "calls page error");
    }
}
