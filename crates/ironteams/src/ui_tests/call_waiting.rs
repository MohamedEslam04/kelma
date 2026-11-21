//! Call waiting driven through the real window: a meeting runs (the fake call driver), a 1:1 call
//! rings over it (the fake invitation), and the toast's Hold and accept, End and accept and
//! Decline, then the "On hold" bar's Resume and End, are clicked. Nothing reaches the network or
//! a sound card; the fake driver records what each call was asked.

use super::*;
use crate::backend::call::{CallCmd, CallKind, Event, FakeDriver};
use crate::backend::call_direct::{CallCmd as DirectCmd, IncomingState, is_direct_chat};
use crate::{CallStore, DirectCallStore};
use i_slint_backend_testing::AccessibleRole;

const PEER: &str = "8:orgid:00000000-0000-0000-0000-000000000002";
const THREAD: &str = "19:meeting_abc@thread.v2";
const RESUME: &str = "Resume the call on hold";
const END_HELD: &str = "End the call on hold";

struct Waiting {
    h: Harness,
    /// The meeting's driver.
    fake: FakeDriver,
}

impl Waiting {
    /// The call window and the toast, both wired to the offline engine.
    fn new() -> Self {
        let h = Harness::new();
        let fake = FakeDriver::manual();
        h.engine.calls.set_driver(Arc::new(fake.clone()));
        crate::ui::call::wire(&h.app);
        let run = h.runner();
        h.app.global::<CallStore>().on_command({
            let run = run.clone();
            move |name, arg| {
                if let Some(c) = CallCmd::parse(&name, &arg) {
                    run(Cmd::Call(c));
                }
            }
        });
        {
            let d = h.app.global::<DirectCallStore>();
            d.set_available(true);
            d.on_is_direct(|id| is_direct_chat(&id));
            d.on_command(move |name, arg| {
                if let Some(c) = DirectCmd::parse(&name, &arg) {
                    run(Cmd::DirectCall(c));
                }
            });
        }
        Waiting { h, fake }
    }

    fn cs(&self) -> CallStore<'_> {
        self.h.app.global::<CallStore>()
    }

    fn ds(&self) -> DirectCallStore<'_> {
        self.h.app.global::<DirectCallStore>()
    }

    /// A connected meeting, as the meeting chat's Join starts it.
    fn in_meeting(&self) {
        self.h.app.set_current_id(THREAD.into());
        click(&self.button("Join meeting"));
        click(&self.button("Join now"));
        self.fake.emit(Event::Connected);
        assert_eq!(self.cs().get_phase(), "connected");
    }

    fn ring(&self, name: &str) {
        let _rt = self.h.rt.enter();
        self.h.engine.fake_incoming_call(PEER, name, false);
    }

    fn buttons(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.h.app, label)
            .filter(|e| e.accessible_role() == Some(AccessibleRole::Button))
            .collect()
    }

    fn button(&self, label: &str) -> ElementHandle {
        self.buttons(label)
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no button {label:?}"))
    }

    fn shows(&self, label: &str) -> bool {
        ElementHandle::find_by_accessible_label(&self.h.app, label)
            .next()
            .is_some()
    }

    fn title(&self) -> String {
        self.cs().get_title().to_string()
    }

    fn held_name(&self) -> String {
        self.cs().get_held_name().to_string()
    }

    /// The meeting on hold, the 1:1 call with Sara shown.
    fn hold_and_accept(&self) {
        self.in_meeting();
        self.ring("Sara Example");
        click(&self.button("Hold and accept"));
        assert_eq!(self.title(), "Sara Example");
    }
}

#[test]
fn a_call_over_a_call_offers_hold_and_accept_end_and_accept_and_decline() {
    let w = Waiting::new();
    w.in_meeting();
    w.ring("Sara Example");
    assert!(w.ds().get_incoming_open());
    assert!(w.ds().get_incoming_waiting());
    assert!(w.shows("Incoming call · You're in a call"));
    for label in ["Hold and accept", "End and accept", "Decline"] {
        assert_eq!(w.buttons(label).len(), 1, "one {label:?}");
    }
    assert!(w.buttons("Accept with audio").is_empty());
    assert!(w.buttons("Accept with video").is_empty());
    assert_eq!(
        w.h.engine.incoming_call_state(),
        Some(IncomingState::Ringing)
    );
    assert!(!w.h.engine.ringtone_playing(), "tests never beep");
}

#[test]
fn without_a_call_the_toast_is_the_plain_one() {
    let w = Waiting::new();
    w.ring("Sara Example");
    assert!(!w.ds().get_incoming_waiting());
    assert_eq!(w.buttons("Accept with audio").len(), 1);
    assert!(w.buttons("Hold and accept").is_empty());
}

#[test]
fn hold_and_accept_holds_the_meeting_and_shows_the_new_call() {
    let w = Waiting::new();
    w.hold_and_accept();
    assert_eq!(w.h.engine.call_log(), ["hold and accept audio"]);
    assert!(!w.ds().get_incoming_open());
    assert_eq!(
        w.fake.calls().last().unwrap(),
        "hold on",
        "the meeting is held"
    );
    assert_eq!(w.cs().get_phase(), "connected");
    assert_eq!(w.cs().get_kind(), "direct");
    assert!(!w.cs().get_held(), "the shown call runs");
    assert_eq!(w.held_name(), "Meeting");
    assert!(w.shows("Meeting · On hold"));
    w.button(RESUME);
    w.button(END_HELD);
}

#[test]
fn resume_swaps_the_two_calls() {
    let w = Waiting::new();
    w.hold_and_accept();
    click(&w.button(RESUME));
    assert_eq!(w.title(), "Meeting");
    assert_eq!(w.cs().get_kind(), "meeting");
    assert_eq!(
        w.fake.calls().last().unwrap(),
        "hold off",
        "the meeting resumed"
    );
    assert_eq!(w.held_name(), "Sara Example");
    assert!(w.shows("Sara Example · On hold"));
    click(&w.button(RESUME));
    assert_eq!(w.title(), "Sara Example");
    assert_eq!(w.fake.calls().last().unwrap(), "hold on");
    assert_eq!(w.held_name(), "Meeting");
}

#[test]
fn end_and_accept_leaves_the_meeting() {
    let w = Waiting::new();
    w.in_meeting();
    w.ring("Sara Example");
    click(&w.button("End and accept"));
    assert_eq!(w.h.engine.call_log(), ["end and accept audio"]);
    assert_eq!(w.fake.calls().last().unwrap(), "leave");
    assert!(!w.fake.calls().iter().any(|c| c == "hold on"));
    assert_eq!(w.cs().get_phase(), "connected");
    assert_eq!(w.title(), "Sara Example");
    assert_eq!(w.held_name(), "", "nothing on hold");
}

#[test]
fn decline_keeps_the_meeting_as_it_was() {
    let w = Waiting::new();
    w.in_meeting();
    w.ring("Sara Example");
    click(&w.button("Decline"));
    assert_eq!(w.h.engine.call_log(), ["decline"]);
    assert!(!w.ds().get_incoming_open());
    assert_eq!(w.title(), "Meeting");
    assert!(!w.fake.calls().iter().any(|c| c.starts_with("hold")));
    assert_eq!(w.held_name(), "");
}

#[test]
fn ending_the_shown_call_leaves_the_held_one_on_hold() {
    let w = Waiting::new();
    w.hold_and_accept();
    click(&w.button("Leave"));
    // The meeting is shown again, still on hold, with its own bar.
    assert_eq!(w.cs().get_phase(), "connected");
    assert_eq!(w.title(), "Meeting");
    assert!(w.cs().get_held());
    assert_eq!(w.held_name(), "Meeting");
    assert_eq!(
        w.fake.calls().last().unwrap(),
        "hold on",
        "not resumed by itself"
    );
    click(&w.button(RESUME));
    assert!(!w.cs().get_held());
    assert_eq!(w.held_name(), "");
    assert_eq!(w.fake.calls().last().unwrap(), "hold off");
}

#[test]
fn end_on_the_bar_ends_the_call_on_hold() {
    let w = Waiting::new();
    w.hold_and_accept();
    click(&w.button(END_HELD));
    assert_eq!(w.fake.calls().last().unwrap(), "leave");
    assert_eq!(w.held_name(), "");
    assert_eq!(w.title(), "Sara Example", "the shown call goes on");
    assert_eq!(w.cs().get_phase(), "connected");
}

#[test]
fn the_held_call_ending_on_its_own_takes_the_bar_away() {
    let w = Waiting::new();
    w.hold_and_accept();
    w.fake.emit(Event::Ended("The meeting ended.".into()));
    assert_eq!(w.held_name(), "");
    assert_eq!(w.title(), "Sara Example");
    assert_eq!(w.cs().get_phase(), "connected");
}

#[test]
fn the_floating_call_shows_the_bar_too() {
    let w = Waiting::new();
    w.hold_and_accept();
    w.cs().set_minimized(true);
    assert!(w.shows("Meeting · On hold"));
    click(&w.button(RESUME));
    assert_eq!(w.title(), "Meeting");
}

#[test]
fn a_third_call_is_not_offered_while_one_is_on_hold() {
    let w = Waiting::new();
    w.hold_and_accept();
    w.ring("Omar Example");
    assert!(!w.ds().get_incoming_open());
    assert_eq!(w.h.engine.incoming_call_state(), None);
}

/// Settings covers the call window and the floating card (z-order), so a call that starts while
/// it is open must close it. Live run: the call connected but nothing showed until Settings was
/// left.
fn settle() {
    for _ in 0..3 {
        testing::mock_elapsed_time(std::time::Duration::from_millis(16));
    }
}

#[test]
fn answering_a_call_with_settings_open_brings_the_call_to_the_front() {
    use crate::SettingsStore;
    let w = Waiting::new();
    let settings = w.h.app.global::<SettingsStore>();
    settings.invoke_show("general".into());
    assert!(settings.get_open());
    w.ring("Sara Example");
    settle();
    assert!(settings.get_open(), "a ringing call does not close it");
    click(&w.button("Accept with audio"));
    // What `accept_live` does once the call connects: hands it to the call window.
    w.h.engine.calls.adopt_without_engine(
        CallKind::Direct {
            peer_name: "Sara Example".into(),
            chat_id: String::new(),
        },
        None,
    );
    settle();
    assert_eq!(w.cs().get_phase(), "connected");
    assert!(!settings.get_open(), "Settings gave way to the call");
    assert!(!w.cs().get_minimized(), "the call window is shown");
}

#[test]
fn joining_a_meeting_with_settings_open_shows_the_pre_join_screen() {
    use crate::SettingsStore;
    let w = Waiting::new();
    let settings = w.h.app.global::<SettingsStore>();
    settings.invoke_show("general".into());
    w.h.app.set_current_id(THREAD.into());
    // Settings covers the button; a calendar Join or a link starts the same call.
    w.cs().invoke_command("join-chat".into(), THREAD.into());
    settle();
    assert_eq!(w.cs().get_phase(), "prejoin");
    assert!(!settings.get_open());
}

#[test]
fn settings_can_be_opened_again_during_a_call() {
    use crate::SettingsStore;
    let w = Waiting::new();
    w.in_meeting();
    settle();
    let settings = w.h.app.global::<SettingsStore>();
    settings.invoke_show("devices".into());
    settle();
    assert!(settings.get_open(), "the call's later changes leave it be");
    w.fake.emit(Event::Connected);
    settle();
    assert!(settings.get_open());
}
