//! The call window driven through the real window and engine, with the fake call driver: no
//! network and no audio device. Every `CallStore.command` goes through `CallCmd::parse` and the
//! engine, the fake records what the call was asked, and the tests send the events the live engine
//! would (lobby, connected, roster, end).

use super::*;
use crate::CallStore;
use crate::backend::CalCmd;
use crate::backend::call::{CallCmd, Event, FakeDriver, Person};
use i_slint_backend_testing::AccessibleRole;

const THREAD: &str = "19:meeting_abc@thread.v2";

fn cs(app: &AppWindow) -> CallStore<'_> {
    app.global::<CallStore>()
}

struct CallHarness {
    h: Harness,
    fake: FakeDriver,
    /// Every `name argument` the window sent, in order.
    log: Log,
}

impl CallHarness {
    fn new() -> Self {
        let h = Harness::new();
        let fake = FakeDriver::manual();
        h.engine.calls.set_driver(Arc::new(fake.clone()));
        crate::ui::call::wire(&h.app);
        let log = Log::default();
        let (l, run) = (log.clone(), h.runner());
        cs(&h.app).on_command(move |name, arg| {
            l.borrow_mut()
                .push(format!("{name} {arg}").trim().to_owned());
            if let Some(c) = CallCmd::parse(&name, &arg) {
                run(Cmd::Call(c));
            }
        });
        CallHarness { h, fake, log }
    }

    /// The meeting chat header's Join, as the window shows it for a meeting chat.
    fn open_prejoin(&self) {
        self.h.app.set_current_id(THREAD.into());
        click(&self.button("Join meeting"));
        assert_eq!(self.phase(), "prejoin");
    }

    /// Pre-join, Join now, then the call connects.
    fn connected(&self) {
        self.open_prejoin();
        click(&self.button("Join now"));
        self.fake.emit(Event::Connected);
        assert_eq!(self.phase(), "connected");
    }

    fn phase(&self) -> String {
        cs(&self.h.app).get_phase().to_string()
    }

    fn all(&self, label: &str) -> Vec<ElementHandle> {
        ElementHandle::find_by_accessible_label(&self.h.app, label).collect()
    }

    fn button(&self, label: &str) -> ElementHandle {
        self.all(label)
            .into_iter()
            .find(|e| e.accessible_role() == Some(AccessibleRole::Button))
            .unwrap_or_else(|| panic!("no button {label:?}"))
    }

    fn with_role(&self, label: &str, role: AccessibleRole) -> ElementHandle {
        self.all(label)
            .into_iter()
            .find(|e| e.accessible_role() == Some(role))
            .unwrap_or_else(|| panic!("no {role:?} {label:?}"))
    }

    fn row(&self, label: &str) -> ElementHandle {
        self.with_role(label, AccessibleRole::ListItem)
    }

    fn shows(&self, label: &str) -> bool {
        !self.all(label).is_empty()
    }

    fn sent(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

#[test]
fn the_prejoin_screen_shows_the_meeting_your_name_and_devices() {
    let c = CallHarness::new();
    c.open_prejoin();
    assert_eq!(c.sent(), [format!("join-chat {THREAD}")]);
    assert!(c.shows("Join as Me"));
    assert!(c.shows("Your camera is off"));

    let camera = c.button("Camera");
    assert_eq!(camera.accessible_enabled(), Some(false));
    assert_eq!(
        camera.accessible_description().unwrap(),
        "Video isn't available yet"
    );

    let mic = c
        .all("Microphone")
        .into_iter()
        .find(|e| e.accessible_role() == Some(AccessibleRole::Switch))
        .unwrap();
    assert_eq!(mic.accessible_checked(), Some(true));
    click(&mic);
    assert!(!cs(&c.h.app).get_prejoin_mic());
    assert!(c.shows("Mic off"));

    let pickers: Vec<_> = ElementHandle::find_by_accessible_label(&c.h.app, "Speaker")
        .filter(|e| e.accessible_role() == Some(AccessibleRole::Combobox))
        .collect();
    assert_eq!(pickers.len(), 1);
    assert_eq!(pickers[0].accessible_value().unwrap(), "Built-in Speakers");
    click(&pickers[0]);
    let usb = c
        .all("USB Headset")
        .into_iter()
        .find(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap();
    click(&usb);
    assert_eq!(cs(&c.h.app).get_speaker(), 1);
    assert!(
        c.all("USB Headset")
            .iter()
            .all(|e| e.accessible_role() != Some(AccessibleRole::ListItem)),
        "the list closes after a pick"
    );

    click(&c.button("Cancel"));
    assert_eq!(c.phase(), "");
    assert_eq!(
        c.sent()[1..],
        ["prejoin-mic off", "speaker 1", "cancel"].map(String::from)
    );
    assert!(c.fake.calls().is_empty(), "nothing joined");
}

#[test]
fn joining_goes_through_the_lobby_to_connected() {
    let c = CallHarness::new();
    c.open_prejoin();
    click(&c.button("Join now"));
    assert_eq!(c.fake.calls(), [format!("join chat:{THREAD} mic=on")]);
    assert_eq!(c.phase(), "connecting");
    assert!(c.shows("Connecting…"));

    c.fake.emit(Event::Lobby);
    assert_eq!(c.phase(), "lobby");
    assert!(c.shows("Someone in the meeting should let you in soon"));

    // Admission re-offers the media: connecting again, then in the meeting.
    c.fake.emit(Event::Connecting);
    c.fake.emit(Event::Connected);
    assert_eq!(c.phase(), "connected");
    assert!(!c.shows("Someone in the meeting should let you in soon"));
    assert!(c.shows("Call duration 00:00"));
    assert!(c.shows("You're the only one here"));
    assert!(
        !c.shows("Recording"),
        "hidden unless the call says it is recorded"
    );
    c.fake.emit(Event::Recording(true));
    assert!(c.shows("Recording"));

    c.fake.emit(Event::Reconnecting);
    assert!(c.shows("Reconnecting… Your connection was interrupted."));
    c.fake.emit(Event::Connected);
    assert!(!c.shows("Reconnecting… Your connection was interrupted."));
}

#[test]
fn joining_with_the_mic_off_joins_muted() {
    let c = CallHarness::new();
    c.open_prejoin();
    click(&c.with_role("Microphone", AccessibleRole::Switch));
    click(&c.button("Join now"));
    assert_eq!(c.fake.calls(), [format!("join chat:{THREAD} mic=off")]);
    assert!(cs(&c.h.app).get_muted());
    assert!(c.shows("Unmute"));
}

#[test]
fn mute_and_hand_call_the_engine() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("Mute"));
    assert!(cs(&c.h.app).get_muted());
    assert_eq!(c.button("Unmute").accessible_checked(), Some(true));
    click(&c.button("Unmute"));
    assert!(!cs(&c.h.app).get_muted());

    click(&c.button("Raise"));
    assert!(cs(&c.h.app).get_hand());
    assert_eq!(c.button("Lower").accessible_checked(), Some(true));
    click(&c.button("Lower"));
    assert!(!cs(&c.h.app).get_hand());

    // A hand lowered by an organizer comes back as an event.
    click(&c.button("Raise"));
    c.fake.emit(Event::Hand(false));
    assert!(c.shows("Raise"));

    assert_eq!(
        c.fake.calls()[1..],
        ["mute on", "mute off", "hand up", "hand down", "hand up"].map(String::from)
    );
}

#[test]
fn the_roster_lists_people_with_mute_and_hand_and_the_lobby() {
    let c = CallHarness::new();
    c.connected();
    let p = |name: &str, muted, hand, lobby, me, role: &str| Person {
        name: name.into(),
        muted,
        hand,
        lobby,
        me,
        role: role.into(),
        ..Default::default()
    };
    c.fake.emit(Event::Roster(vec![
        p("Guest", false, false, true, false, ""),
        p("Ada Lovelace", false, false, false, false, "organizer"),
        p("Me", false, false, false, true, "attendee"),
        p("Bo Ström", true, true, false, false, "attendee"),
    ]));
    click(&c.button("People"));
    assert_eq!(c.button("People").accessible_checked(), Some(true));
    assert!(c.shows("In this meeting (3)"));
    assert!(c.shows("Waiting in lobby (1)"));
    let mut rows: Vec<(f32, String, String)> = ["Ada Lovelace", "Me", "Bo Ström", "Guest"]
        .into_iter()
        .map(|n| {
            let e = c.row(n);
            let d = e.accessible_description().unwrap().to_string();
            (e.absolute_position().y, label(&e), d)
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let rows: Vec<(String, String)> = rows.into_iter().map(|(_, n, d)| (n, d)).collect();
    assert_eq!(
        rows,
        [
            ("Ada Lovelace", "Unmuted, organizer"),
            ("Me", "Unmuted, attendee"),
            ("Bo Ström", "Muted, hand raised, attendee"),
            ("Guest", "In lobby"),
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    assert!(c.shows("Me (You)"));

    // Our own row follows our controls.
    click(&c.button("Mute"));
    assert_eq!(
        c.row("Me").accessible_description().unwrap(),
        "Muted, attendee"
    );

    click(&c.button("Close People"));
    assert_eq!(cs(&c.h.app).get_panel(), "");
}

#[test]
fn share_asks_the_engine_and_follows_what_it_reports() {
    let c = CallHarness::new();
    c.connected();
    assert_eq!(c.button("Share").accessible_enabled(), Some(false));
    c.fake.emit(Event::Share {
        available: true,
        sharing: false,
    });
    click(&c.button("Share"));
    // The picker is the desktop's; the button turns only when the engine says it shares.
    assert!(c.shows("Share"));
    c.fake.emit(Event::Share {
        available: true,
        sharing: true,
    });
    assert_eq!(c.button("Stop sharing").accessible_checked(), Some(true));
    click(&c.button("Stop sharing"));
    assert_eq!(
        c.fake.calls()[1..],
        ["share on", "share off"].map(String::from)
    );
}

#[test]
fn camera_asks_the_engine_and_follows_what_it_reports() {
    let c = CallHarness::new();
    c.connected();
    assert_eq!(c.button("Camera").accessible_enabled(), Some(false));
    c.fake.emit(Event::Camera {
        available: true,
        on: false,
    });
    click(&c.button("Camera"));
    // The portal may still refuse; the button turns only when the engine says it is on.
    assert_eq!(c.button("Camera").accessible_checked(), Some(false));
    c.fake.emit(Event::Camera {
        available: true,
        on: true,
    });
    assert_eq!(c.button("Camera").accessible_checked(), Some(true));
    click(&c.button("Camera"));
    assert_eq!(
        c.fake.calls()[1..],
        ["camera on", "camera off"].map(String::from)
    );
}

#[test]
fn the_prejoin_camera_switch_works_when_the_build_has_video() {
    let c = CallHarness::new();
    c.open_prejoin();
    cs(&c.h.app).set_camera_available(true);
    let camera = c.button("Camera");
    assert_eq!(camera.accessible_enabled(), Some(true));
    click(&camera);
    assert!(c.shows("Your camera turns on when you join"));
    assert!(c.sent().contains(&"prejoin-camera on".to_owned()));
}

#[test]
fn unavailable_controls_are_shown_disabled_and_say_why() {
    let c = CallHarness::new();
    c.connected();
    for (label, why) in [
        ("React", "Reactions aren't available yet"),
        ("Camera", "Video isn't available in this call"),
        ("Share", "Screen sharing isn't available in this call"),
    ] {
        let b = c.button(label);
        assert_eq!(b.accessible_enabled(), Some(false), "{label}");
        assert_eq!(b.accessible_description().unwrap(), why);
        click(&b);
    }
    // No meeting chat known for a code join: Chat says so. Here the chat is known.
    assert_eq!(c.button("Chat").accessible_enabled(), Some(true));
    assert_eq!(c.sent().len(), 2, "only join-chat and join-now were sent");
    assert_eq!(c.fake.calls().len(), 1);
}

#[test]
fn a_share_takes_the_stage_and_names_who_presents() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("View"));
    let focus = || {
        ElementHandle::find_by_accessible_label(&c.h.app, "Focus on content")
            .next()
            .expect("the View menu lists Focus on content")
    };
    assert_eq!(focus().accessible_enabled(), Some(false));
    assert_eq!(
        focus().accessible_description().unwrap(),
        "Nobody is sharing"
    );

    c.fake.emit(Event::Presenter("Ada Lovelace".into()));
    assert_eq!(
        cs(&c.h.app).get_view_mode(),
        "content",
        "a share takes the stage"
    );
    assert!(c.shows("Ada Lovelace is presenting"));

    c.fake.emit(Event::Presenter(String::new()));
    assert_eq!(cs(&c.h.app).get_view_mode(), "gallery", "and gives it back");
}

#[test]
fn more_and_the_mic_dropdown_switch_devices_in_the_call() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("Audio devices"));
    assert!(c.shows("Audio devices"));
    let mic = ElementHandle::find_by_accessible_label(&c.h.app, "Microphone")
        .find(|e| e.accessible_role() == Some(AccessibleRole::Combobox))
        .unwrap();
    click(&mic);
    let usb = c
        .all("USB Headset Microphone")
        .into_iter()
        .find(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap();
    click(&usb);
    assert_eq!(
        c.fake.calls().last().unwrap(),
        "devices mic-usb spk-builtin"
    );
    click(&c.button("More"));
    assert_eq!(cs(&c.h.app).get_panel(), "more");
    assert!(c.shows("Device settings"));
}

#[test]
fn leave_shows_the_call_ended_screen_and_close_dismisses_it() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("Leave"));
    assert_eq!(c.fake.calls().last().unwrap(), "leave");
    assert_eq!(c.phase(), "ended");
    assert!(c.shows("You left the meeting."));
    // The driver's own end report after we left changes nothing.
    c.fake.emit(Event::Ended("The meeting ended.".into()));
    assert!(c.shows("You left the meeting."));

    click(&c.button("Rejoin"));
    assert_eq!(c.phase(), "prejoin");
    click(&c.button("Join now"));
    c.fake.emit(Event::Ended("The meeting ended.".into()));
    assert!(c.shows("The meeting ended."));
    click(&c.button("Close"));
    assert_eq!(c.phase(), "");
}

#[test]
fn join_with_an_id_opens_the_prejoin_for_that_meeting() {
    let c = CallHarness::new();
    c.h.runner()(Cmd::Calendar(CalCmd::JoinId {
        id: "123 456 789".into(),
        passcode: " pw ".into(),
    }));
    assert_eq!(c.phase(), "prejoin");
    assert_eq!(cs(&c.h.app).get_title(), "Meeting 123456789");
    click(&c.button("Join now"));
    assert_eq!(c.fake.calls(), ["join code mic=on"]);
    // No chat is known for a code: the Chat button says why.
    c.fake.emit(Event::Connected);
    let chat = c.button("Chat");
    assert_eq!(chat.accessible_enabled(), Some(false));
    assert_eq!(
        chat.accessible_description().unwrap(),
        "This meeting's chat isn't available here"
    );
}

#[test]
fn chat_shrinks_the_call_to_a_bar_and_opens_the_meeting_chat() {
    let c = CallHarness::new();
    c.connected();
    let opened = Log::default();
    let o = opened.clone();
    c.h.app
        .on_select_chat(move |id| o.borrow_mut().push(id.to_string()));
    click(&c.button("Chat"));
    assert!(cs(&c.h.app).get_minimized());
    assert_eq!(*opened.borrow(), [THREAD.to_owned()]);
    assert_eq!(c.h.app.get_view(), "chat");
    click(&c.button("Back to call"));
    assert!(!cs(&c.h.app).get_minimized());
}

#[test]
fn the_rail_leaves_the_call_view_and_keeps_the_call_in_its_bar() {
    let c = CallHarness::new();
    c.connected();
    let mut rail: Vec<ElementHandle> =
        ElementHandle::find_by_element_id(&c.h.app, "AppWindow::ri").collect();
    rail.sort_by(|a, b| a.absolute_position().y.total_cmp(&b.absolute_position().y));
    // Chat is already the view under the call: clicking it still shows the chats.
    rail[1].mock_single_click(PointerEventButton::Left);
    assert!(cs(&c.h.app).get_minimized());
    assert_eq!(c.h.app.get_view(), "chat");
    click(&c.button("Back to call"));
    rail[0].mock_single_click(PointerEventButton::Left);
    assert!(cs(&c.h.app).get_minimized());
    assert_eq!(c.h.app.get_view(), "activity");
    assert_eq!(c.phase(), "connected", "the call keeps running");
}

#[test]
fn the_floating_call_shows_the_speaker_or_more_people() {
    use slint::Model;
    let c = CallHarness::new();
    c.connected();
    let p = |name: &str, me, spoke| Person {
        name: name.into(),
        me,
        spoke,
        speaking: spoke == 2,
        ..Default::default()
    };
    c.fake.emit(Event::Roster(vec![
        p("Ada Lovelace", false, 1),
        p("Me", true, 0),
        p("Bo Ström", false, 2),
    ]));
    cs(&c.h.app).set_minimized(true);
    let mini = cs(&c.h.app).get_mini_people();
    let names: Vec<String> = mini.iter().map(|p| p.name.to_string()).collect();
    assert_eq!(
        names,
        ["Bo Ström", "Ada Lovelace"],
        "latest speaker first, without us"
    );
    assert!(mini.row_data(0).unwrap().speaking);
    assert_eq!(cs(&c.h.app).get_mini_layout(), "speaker");
    click(&c.button("Show more people"));
    assert_eq!(cs(&c.h.app).get_mini_layout(), "gallery");
    click(&c.button("Show the speaker"));
    assert_eq!(cs(&c.h.app).get_mini_layout(), "speaker");
}

/// A roster of three, Bo Ström the latest to speak.
fn view_roster(c: &CallHarness) {
    let p = |name: &str, me, spoke| Person {
        name: name.into(),
        me,
        spoke,
        speaking: spoke == 2,
        ..Default::default()
    };
    c.fake.emit(Event::Roster(vec![
        p("Ada Lovelace", false, 1),
        p("Me", true, 0),
        p("Bo Ström", false, 2),
    ]));
}

fn view_item(c: &CallHarness, label: &str) -> ElementHandle {
    c.with_role(label, AccessibleRole::ListItem)
}

#[test]
fn the_view_menu_switches_between_gallery_and_speaker() {
    let c = CallHarness::new();
    c.connected();
    view_roster(&c);
    assert_eq!(cs(&c.h.app).get_view_mode(), "gallery");
    let view = c.button("View");
    assert_eq!(view.accessible_enabled(), Some(true));

    click(&view);
    assert_eq!(cs(&c.h.app).get_menu(), "view");
    assert_eq!(
        view_item(&c, "Gallery").accessible_item_selected(),
        Some(true)
    );
    assert_eq!(
        view_item(&c, "Speaker").accessible_item_selected(),
        Some(false)
    );
    click(&view_item(&c, "Speaker"));
    assert_eq!(cs(&c.h.app).get_view_mode(), "speaker");
    assert_eq!(cs(&c.h.app).get_menu(), "", "picking closes the menu");

    click(&c.button("View"));
    assert_eq!(
        view_item(&c, "Speaker").accessible_item_selected(),
        Some(true)
    );
    click(&view_item(&c, "Gallery"));
    assert_eq!(cs(&c.h.app).get_view_mode(), "gallery");
}

#[test]
fn focus_on_content_is_disabled_and_says_nobody_is_sharing() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("View"));
    let focus = view_item(&c, "Focus on content");
    assert_eq!(focus.accessible_enabled(), Some(false));
    assert_eq!(focus.accessible_description().unwrap(), "Nobody is sharing");
    click(&focus);
    assert_eq!(cs(&c.h.app).get_view_mode(), "gallery");
}

#[test]
fn the_speaker_view_shows_the_latest_speaker_large_and_the_others_below() {
    let c = CallHarness::new();
    c.connected();
    view_roster(&c);
    assert!(
        !c.shows("Active speaker: Bo Ström"),
        "the gallery has no large tile"
    );
    cs(&c.h.app).set_view_mode("speaker".into());
    let big = c.with_role("Active speaker: Bo Ström", AccessibleRole::Text);
    let big_bottom = big.absolute_position().y + big.size().height;
    for other in ["Ada Lovelace", "Me (You)"] {
        let y = c.all(other)[0].absolute_position().y;
        assert!(
            y >= big_bottom,
            "{other} is in the strip below the large tile"
        );
    }
    assert_eq!(c.all("Bo Ström").len(), 1, "the speaker is not shown twice");

    // Somebody else speaks: the large tile follows.
    c.fake.emit(Event::Roster(vec![
        Person {
            name: "Ada Lovelace".into(),
            spoke: 3,
            ..Default::default()
        },
        Person {
            name: "Bo Ström".into(),
            spoke: 2,
            ..Default::default()
        },
    ]));
    assert!(c.shows("Active speaker: Ada Lovelace"));
    assert!(!c.shows("Active speaker: Bo Ström"));
}

#[test]
fn photos_replace_the_initials_of_the_people_who_have_one() {
    use slint::Model;
    let c = CallHarness::new();
    c.connected();
    c.h.engine.test_set_avatar(
        "8:orgid:example-ada",
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(4, 4),
    );
    let p = |name: &str, mri: &str, spoke| Person {
        name: name.into(),
        mri: mri.into(),
        spoke,
        ..Default::default()
    };
    c.fake.emit(Event::Roster(vec![
        p("Ada Lovelace", "8:orgid:example-ada", 5),
        p("Bo Ström", "8:orgid:example-bo", 1),
    ]));
    let has_photo = |m: slint::ModelRc<crate::CallPerson>| -> Vec<(String, bool)> {
        m.iter()
            .map(|p| (p.name.to_string(), p.has_photo))
            .collect()
    };
    let expected = [
        ("Ada Lovelace".to_owned(), true),
        ("Bo Ström".to_owned(), false),
    ];
    assert_eq!(has_photo(cs(&c.h.app).get_people()), expected);
    assert_eq!(has_photo(cs(&c.h.app).get_speaker_people()), expected);
    assert_eq!(has_photo(cs(&c.h.app).get_mini_people()), expected);
}

#[test]
fn a_second_join_while_in_a_call_is_refused() {
    let c = CallHarness::new();
    c.connected();
    c.h.runner()(Cmd::Calendar(CalCmd::JoinId {
        id: "1".into(),
        passcode: String::new(),
    }));
    assert_eq!(c.phase(), "connected");
    assert!(c.shows(crate::backend::call::ALREADY_IN_CALL));
    click(&c.button("Dismiss"));
    assert!(!c.shows(crate::backend::call::ALREADY_IN_CALL));
}

const RECORDING_NOTICE: &str =
    "Recording has started. By attending this meeting, you consent to being included.";

#[test]
fn more_starts_and_stops_recording_and_transcription() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("More"));
    assert!(c.shows("Record and transcribe"));
    assert!(!c.shows("Recording"), "no recording dot yet");

    click(&c.button("Start recording"));
    assert_eq!(c.sent().last().unwrap(), "recording start");
    assert_eq!(c.fake.calls().last().unwrap(), "recording start");
    // The state comes from the meeting, with Teams' consent notice.
    c.fake.emit(Event::Recording(true));
    assert!(c.shows("Recording"), "the recording dot");
    assert!(c.shows(RECORDING_NOTICE));
    click(&c.button("Dismiss notice"));
    assert!(!c.shows(RECORDING_NOTICE));
    click(&c.button("Stop recording"));
    assert_eq!(c.fake.calls().last().unwrap(), "recording stop");
    c.fake.emit(Event::Recording(false));
    assert!(c.shows("Start recording"));

    click(&c.button("Start transcription"));
    assert_eq!(c.fake.calls().last().unwrap(), "transcription start");
    c.fake.emit(Event::Transcribing(true));
    assert!(c.shows(
        "Transcription has started. By attending this meeting, you consent to being included."
    ));
    click(&c.button("Stop transcription"));
    assert_eq!(c.fake.calls().last().unwrap(), "transcription stop");
}

#[test]
fn a_refused_recording_shows_the_services_reason() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("More"));
    click(&c.button("Start recording"));
    let why = "Couldn't start recording: Recording is disabled by policy (403/10501)";
    c.fake.emit(Event::Error(why.into()));
    assert!(c.shows(why));
    assert!(c.shows("Start recording"), "nothing started");
}

#[test]
fn live_captions_show_the_speaker_and_the_text_at_the_bottom() {
    let c = CallHarness::new();
    c.connected();
    click(&c.button("More"));
    let switch = c.with_role("Show live captions", AccessibleRole::Switch);
    assert_eq!(switch.accessible_checked(), Some(false));
    click(&switch);
    assert_eq!(c.fake.calls().last().unwrap(), "captions on");
    assert_eq!(
        c.with_role("Show live captions", AccessibleRole::Switch)
            .accessible_checked(),
        Some(true)
    );
    assert!(c.shows("Waiting for someone to speak…"));

    let caption = |id: &str, speaker: &str, text: &str| {
        Event::Caption(crate::backend::call::Caption {
            id: id.into(),
            speaker: speaker.into(),
            text: text.into(),
        })
    };
    c.fake.emit(caption("u1", "Ada Example", "Hel"));
    c.fake.emit(caption("u1", "Ada Example", "Hello everyone"));
    c.fake.emit(caption("u2", "Bo Example", "Hi"));
    assert!(c.shows("Ada Example: Hello everyone"));
    assert!(c.shows("Bo Example: Hi"));
    assert!(!c.shows("Ada Example: Hel"), "replaced in place");
    assert!(!c.shows("Waiting for someone to speak…"));

    // Turned off (or refused by the service): the strip goes away.
    c.fake.emit(Event::Captions(false));
    assert!(!c.shows("Bo Example: Hi"));
    assert_eq!(
        c.with_role("Show live captions", AccessibleRole::Switch)
            .accessible_checked(),
        Some(false)
    );
    click(&c.with_role("Show live captions", AccessibleRole::Switch));
    click(&c.with_role("Show live captions", AccessibleRole::Switch));
    assert_eq!(c.fake.calls().last().unwrap(), "captions off");
}

const DM: &str =
    "19:00000000-0000-0000-0000-000000000001_00000000-0000-0000-0000-000000000002@unq.gbl.spaces";
const BO: &str = "8:orgid:00000000-0000-0000-0000-000000000003";

impl CallHarness {
    /// A connected 1:1 call with Ada, as the ringing flows hand it over.
    fn direct(&self) {
        use crate::backend::call::CallKind;
        let kind = CallKind::Direct {
            peer_name: "Ada Example".into(),
            chat_id: DM.into(),
        };
        assert!(self.h.engine.calls.adopt_fake(&self.fake, kind));
        assert_eq!(self.phase(), "connected");
        assert_eq!(cs(&self.h.app).get_kind(), "direct");
    }

    /// Types into the search box labelled `label`; the search answers with Bo.
    fn search(&self, label: &str, text: &str) {
        use crate::backend::call::Found;
        self.with_role(label, AccessibleRole::TextInput)
            .set_accessible_value(text);
        self.h.engine.calls.test_found(vec![Found {
            mri: BO.into(),
            name: "Bo Example".into(),
            subtitle: "Engineer".into(),
        }]);
    }
}

#[test]
fn chat_in_a_direct_call_opens_the_one_on_one_chat_and_shrinks_the_call() {
    let c = CallHarness::new();
    c.direct();
    let opened = Log::default();
    let o = opened.clone();
    c.h.app
        .on_select_chat(move |id| o.borrow_mut().push(id.to_string()));
    let chat = c.button("Chat");
    assert_eq!(chat.accessible_enabled(), Some(true));
    assert_eq!(
        chat.accessible_description().unwrap(),
        "Open the chat with Ada Example"
    );
    click(&chat);
    assert!(cs(&c.h.app).get_minimized());
    assert_eq!(*opened.borrow(), [DM.to_owned()]);
    assert_eq!(c.h.app.get_view(), "chat");
}

#[test]
fn hold_toggles_to_resume_and_back() {
    let c = CallHarness::new();
    c.direct();
    click(&c.button("Hold"));
    assert!(cs(&c.h.app).get_held());
    assert!(c.shows("You put the call on hold. Select Resume to continue."));
    let resume = c.button("Resume");
    assert_eq!(resume.accessible_checked(), Some(true));
    click(&resume);
    assert!(!cs(&c.h.app).get_held());
    assert!(c.shows("Hold"));
    assert_eq!(c.fake.calls(), ["hold on", "hold off"]);
    assert_eq!(c.sent(), ["hold", "hold"]);

    // A hold the service refused puts the button back.
    click(&c.button("Hold"));
    c.fake.emit(Event::Held(false));
    assert!(c.shows("Hold") && !cs(&c.h.app).get_held());
}

#[cfg(feature = "calls")]
#[test]
fn a_refused_hold_re_offer_keeps_the_call_and_hold_available() {
    use crate::backend::call::live::Reoffer;
    let c = CallHarness::new();
    c.direct();
    click(&c.button("Hold"));
    assert!(cs(&c.h.app).get_held());
    // `call/rejection` to the hold re-offer (488/3102): the call goes on with the hold undone.
    for ev in Reoffer::Hold(true).refused() {
        c.fake.emit(ev);
    }
    assert_eq!(c.phase(), "connected");
    assert!(!cs(&c.h.app).get_held());
    assert!(c.shows("Couldn't put the call on hold."));
    click(&c.button("Hold"));
    assert!(cs(&c.h.app).get_held());
    assert_eq!(c.fake.calls(), ["hold on", "hold on"]);
}

#[test]
fn the_invite_box_rings_a_person_into_the_call() {
    let c = CallHarness::new();
    c.direct();
    click(&c.button("People"));
    assert!(c.shows("Currently in this call (0)"));
    c.search("Invite someone or dial a number", "bo");
    assert_eq!(c.sent(), ["find bo"]);
    click(&c.row("Bo Example"));
    assert_eq!(c.fake.calls(), [format!("add {BO}")]);
    assert_eq!(c.sent().last().unwrap(), &format!("invite {BO}"));
    assert!(c.shows("Calling Bo Example…"));
    assert!(
        c.all("Bo Example")
            .iter()
            .all(|e| e.accessible_role() != Some(AccessibleRole::ListItem)),
        "the results close after a pick"
    );
}

#[test]
fn transfer_holds_the_call_then_transfers_now_to_the_picked_person() {
    let c = CallHarness::new();
    c.direct();
    click(&c.button("Transfer"));
    assert!(c.shows("The call is on hold. Pick who takes it."));
    assert!(cs(&c.h.app).get_held(), "opening Transfer holds the call");
    assert_eq!(c.fake.calls(), ["hold on"]);
    assert!(c.all("Transfer now").is_empty(), "nobody picked yet");
    c.search("Transfer to", "bo");
    click(&c.row("Bo Example"));
    assert!(c.shows("To: Bo Example"));
    click(&c.button("Transfer now"));
    assert_eq!(
        c.fake.calls(),
        ["hold on", format!("transfer {BO}").as_str()]
    );
    assert_eq!(cs(&c.h.app).get_menu(), "", "the dialog closes");
    c.fake.emit(Event::Ended(
        "You transferred the call to Bo Example.".into(),
    ));
    assert_eq!(c.phase(), "ended");
}

#[test]
fn a_meeting_has_no_hold_transfer_or_invite_box() {
    let c = CallHarness::new();
    c.connected();
    assert!(c.all("Hold").is_empty() && c.all("Transfer").is_empty());
    click(&c.button("People"));
    assert!(c.all("Invite someone or dial a number").is_empty());
    assert!(c.shows("In this meeting (0)"));
}
