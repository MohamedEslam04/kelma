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
}
