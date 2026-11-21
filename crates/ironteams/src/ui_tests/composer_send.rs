//! The send button's options, driven through the real window: right-click Send or its chevron
//! for "Schedule message" and "Set delivery options" (Standard, Important, Urgent); the picked
//! send time's banner; the scheduled "Sending on …" bubbles with Edit and Delete. The engine is
//! offline: a schedule is kept here only (the account call is skipped).

use super::*;
use crate::SendStore;
use crate::backend::ScheduledCmd;
use crate::send_options::{day_label, month_title, resolve, time_label, when_label};
use chrono::{Datelike, Local};
use i_slint_backend_testing::AccessibleRole;
use teams_proto::delivery::Importance;

fn buttons(app: &AppWindow, label: &str) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label)
        .filter(|e| e.accessible_role() == Some(AccessibleRole::Button))
        .collect()
}

fn button(app: &AppWindow, label: &str) -> ElementHandle {
    let mut b = buttons(app, label);
    assert_eq!(b.len(), 1, "one '{label}' button");
    b.remove(0)
}

fn ss(app: &AppWindow) -> SendStore<'_> {
    app.global::<SendStore>()
}

fn right_click(app: &AppWindow, e: &ElementHandle) {
    let p = e.absolute_position();
    let s = e.size();
    let at = LogicalPosition::new(p.x + s.width / 2., p.y + s.height / 2.);
    app.window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    for pressed in [true, false] {
        app.window().dispatch_event(if pressed {
            WindowEvent::PointerPressed {
                position: at,
                button: PointerEventButton::Right,
            }
        } else {
            WindowEvent::PointerReleased {
                position: at,
                button: PointerEventButton::Right,
            }
        });
    }
}

/// `SendStore` commands go to the engine (as `main.rs` wires them) and are recorded.
fn wire(h: &Harness) -> Log {
    let log = Log::default();
    let run = h.runner();
    let l = log.clone();
    ss(&h.app).on_command(move |name, arg| {
        l.borrow_mut()
            .push(format!("{name} {}", arg.replace('\t', " | ")));
        if let Some(c) = ScheduledCmd::parse(&name, &arg) {
            run(Cmd::Scheduled(c));
        }
    });
    log
}

/// Sends from the window go to the engine with the delivery flags, as `main.rs` does.
fn wire_send(h: &Harness) {
    let run = h.runner();
    let weak = h.app.as_weak();
    h.app.on_send(move |t| {
        let w = weak.upgrade().unwrap();
        if w.get_important() {
            run(Cmd::SetImportant(true));
        }
        if w.get_urgent() {
            run(Cmd::SetUrgent(true));
        }
        run(Cmd::Send(t.into(), String::new()));
    });
}

#[test]
fn right_click_on_send_and_its_chevron_open_the_options() {
    let app = app_with_messages(vec![]);
    assert!(buttons(&app, "Schedule message").is_empty());
    right_click(&app, &button(&app, "Send"));
    button(&app, "Schedule message");
    button(&app, "Set delivery options");
    // Escape-free close: a click elsewhere.
    click(&composer(&app));
    assert!(buttons(&app, "Schedule message").is_empty());
    click(&button(&app, "Send options"));
    button(&app, "Schedule message");
}

#[test]
fn delivery_options_choose_standard_important_or_urgent_for_the_next_message() {
    let app = app_with_messages(vec![]);
    click(&button(&app, "Send options"));
    click(&button(&app, "Set delivery options"));
    let urgent = button(&app, "Urgent");
    assert_eq!(
        urgent.accessible_description().unwrap(),
        "Recipients are notified every 2 minutes for 20 minutes"
    );
    assert_eq!(button(&app, "Standard").accessible_checked(), Some(true));
    click(&urgent);
    assert!(app.get_urgent() && !app.get_important());
    assert!(!find_label(&app, "Urgent message").is_empty(), "the chip");
    assert!(!find_label(&app, "URGENT!").is_empty());

    click(&button(&app, "Send options"));
    click(&button(&app, "Set delivery options"));
    assert_eq!(button(&app, "Urgent").accessible_checked(), Some(true));
    click(&button(&app, "Important"));
    assert!(app.get_important() && !app.get_urgent());

    click(&button(&app, "Send options"));
    click(&button(&app, "Set delivery options"));
    click(&button(&app, "Standard"));
    assert!(!app.get_important() && !app.get_urgent());
}

#[test]
fn an_urgent_message_is_queued_as_urgent_once_and_shows_the_label() {
    let h = Harness::new();
    wire_send(&h);
    h.select(NOTES);
    click(&button(&h.app, "Send options"));
    click(&button(&h.app, "Set delivery options"));
    click(&button(&h.app, "Urgent"));
    h.type_in_composer("come now");
    enter(&h.app);
    assert!(!h.app.get_urgent(), "used up by the send");
    h.type_in_composer("and later");
    enter(&h.app);
    let queued = h.engine.test_store(|st| st.outbox_for(NOTES).unwrap());
    let by_text = |t: &str| queued.iter().find(|i| i.html.contains(t)).unwrap();
    assert_eq!(by_text("come now").importance, "urgent");
    assert_eq!(by_text("and later").importance, "");
    let urgent: Vec<_> = h
        .app
        .get_messages()
        .iter()
        .filter(|m| m.importance == "urgent")
        .map(|m| m.plain.to_string())
        .collect();
    assert_eq!(urgent, ["come now"]);
    for id in h
        .app
        .get_messages()
        .iter()
        .filter(|m| m.id.starts_with("pending-"))
        .map(|m| m.id.to_string())
        .collect::<Vec<_>>()
    {
        h.runner()(Cmd::DiscardSend(id));
    }
}

#[test]
fn the_engine_takes_urgent_like_important_for_one_message() {
    let h = Harness::new();
    h.select(NOTES);
    let run = h.runner();
    run(Cmd::SetImportant(true));
    run(Cmd::SetUrgent(true));
    run(Cmd::Send("last one wins".into(), String::new()));
    let queued = h.engine.test_store(|st| st.outbox_for(NOTES).unwrap());
    assert_eq!(queued[0].importance, Importance::Urgent.wire());
    for m in h
        .app
        .get_messages()
        .iter()
        .filter(|m| m.id.starts_with("pending-"))
    {
        run(Cmd::DiscardSend(m.id.to_string()));
    }
}

#[test]
fn schedule_message_picks_tomorrow_at_eight_and_shows_the_banner() {
    let h = Harness::new();
    let log = wire(&h);
    h.select(BACKEND);
    click(&button(&h.app, "Send options"));
    click(&button(&h.app, "Schedule message"));
    assert!(!find_label(&h.app, "Send this message on").is_empty());
    assert_eq!(ss(&h.app).get_pick_day(), 1);
    assert_eq!(ss(&h.app).get_pick_minutes(), 480);
    assert_eq!(time_value(&h.app), time_label(480));
    // Later by an hour, then back.
    click(&button(&h.app, "Later hour"));
    assert_eq!(ss(&h.app).get_pick_minutes(), 540);
    click(&button(&h.app, "Earlier hour"));
    assert_eq!(ss(&h.app).get_pick_minutes(), 480);
    click(&button(&h.app, "Continue"));
    let at = ss(&h.app).get_schedule_at().to_string();
    let at_ms: i64 = at.parse().expect("a time");
    assert!(at_ms > teams_proto::time::now_ms());
    let label = ss(&h.app).get_schedule_label().to_string();
    assert!(
        label.ends_with(&format!(" at {}", time_label(480))),
        "{label}"
    );
    let banner = format!("Select Send to deliver on {label}");
    assert!(!find_label(&h.app, &banner).is_empty());

    // The clear (x) forgets the time.
    click(&button(&h.app, "Don't schedule"));
    assert_eq!(ss(&h.app).get_schedule_at(), "");
    assert!(find_label(&h.app, &banner).is_empty());
    assert!(log.borrow().is_empty());
}

#[test]
fn a_time_in_the_past_cannot_be_picked() {
    let h = Harness::new();
    h.select(BACKEND);
    click(&button(&h.app, "Send options"));
    click(&button(&h.app, "Schedule message"));
    // Today at midnight is always past.
    ss(&h.app).set_pick_day(0);
    ss(&h.app).set_pick_minutes(0);
    assert!(!find_label(&h.app, "Pick a time in the future").is_empty());
    let cont = button(&h.app, "Continue");
    assert_eq!(cont.accessible_enabled(), Some(false));
    click(&cont);
    assert_eq!(ss(&h.app).get_schedule_at(), "");
}

#[test]
fn send_with_a_time_schedules_and_the_bubble_offers_edit_and_delete() {
    let h = Harness::new();
    let log = wire(&h);
    let sent = record_send(&h.app);
    h.select(BACKEND);
    let at = teams_proto::time::now_ms() + 86_400_000;
    ss(&h.app).set_schedule_at(at.to_string().into());
    ss(&h.app).set_schedule_label(crate::send_options::when_label(at).into());
    h.type_in_composer("see you tomorrow");
    enter(&h.app);
    assert!(sent.borrow().is_empty(), "not sent now");
    assert_eq!(*log.borrow(), [format!("schedule {at} | see you tomorrow")]);
    assert_eq!(h.app.get_draft(), "");
    assert_eq!(ss(&h.app).get_schedule_at(), "", "used up");
    let waiting = h.engine.test_scheduled();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].conversation_id, BACKEND);
    assert_eq!(waiting[0].send_at_ms, at);
    assert!(waiting[0].html.contains("see you tomorrow"));

    // The bubble in the thread.
    let label = crate::send_options::sending_label(at);
    let bubble = find_label(&h.app, &label);
    assert_eq!(bubble.len(), 1, "one bubble saying {label}");
    assert_eq!(
        bubble[0].accessible_description().unwrap(),
        "see you tomorrow"
    );
    // Another chat has no bubble; coming back shows it again.
    h.select(NOTES);
    assert!(find_label(&h.app, &label).is_empty());
    h.select(BACKEND);
    assert_eq!(find_label(&h.app, &label).len(), 1);

    // Edit: back in the composer with the same time chosen, the waiting copy gone.
    click(&button(&h.app, "Edit"));
    assert_eq!(h.app.get_draft(), "see you tomorrow");
    assert_eq!(ss(&h.app).get_schedule_at(), at.to_string());
    assert!(h.engine.test_scheduled().is_empty());
    assert!(find_label(&h.app, &label).is_empty());

    // Send again: scheduled anew; Delete cancels it.
    composer(&h.app).mock_single_click(PointerEventButton::Left);
    enter(&h.app);
    assert_eq!(h.engine.test_scheduled().len(), 1);
    click(&button(&h.app, "Delete"));
    assert!(h.engine.test_scheduled().is_empty());
    assert!(find_label(&h.app, &label).is_empty());
}

#[test]
fn channel_posts_and_replies_are_not_scheduled() {
    let app = app_with_messages(vec![]);
    app.set_current_is_channel(true);
    click(&button(&app, "Send options"));
    assert!(buttons(&app, "Schedule message").is_empty());
    button(&app, "Set delivery options");
    click(&composer(&app));
    // A reply in a thread.
    app.set_current_is_channel(false);
    app.set_thread_root("1700000000000".into());
    click(&button(&app, "Send options"));
    assert!(buttons(&app, "Schedule message").is_empty());
}

/// The account's list as `GET /v1/users/ME/drafts?view=msnp24Equivalent` gives it: the message
/// fields sit at the top level of each draft.
fn listed(
    conv: &str,
    cmid: &str,
    html: &str,
    at: i64,
) -> Vec<teams_proto::scheduled::ScheduledMessage> {
    teams_proto::scheduled::parse_scheduled(&serde_json::json!({"drafts": [{
        "id": format!("d-{cmid}"),
        "draftType": "ScheduledDraft",
        "innerThreadId": conv,
        "draftDetails": {"sendAt": at.to_string()},
        "content": html,
        "clientmessageid": cmid,
        "messagetype": "RichText/Html",
        "properties": {},
    }]}))
}

/// The bubble saying `label` carries `text` (what it reads out and what its text line shows) and
/// both its lines (the "Sending on …" line and the text) are drawn with a size.
fn bubble_shows(app: &AppWindow, label: &str, text: &str) -> bool {
    let bubble = find_label(app, label);
    assert_eq!(bubble.len(), 1, "one bubble saying {label}");
    let items = ss(app).get_scheduled();
    let shown: Vec<String> = items.iter().map(|s| s.text.to_string()).collect();
    let lines = super::overflow::texts(&bubble[0]);
    bubble[0].accessible_description().as_deref() == Some(text)
        && shown == [text]
        && lines.len() == 2
        && lines
            .iter()
            .all(|t| t.size().height > 0. && t.size().width > 0.)
}

#[test]
fn a_listed_scheduled_message_shows_its_text_in_the_bubble() {
    let h = Harness::new();
    h.select(BACKEND);
    let at = teams_proto::time::now_ms() + 86_400_000;
    let text = "this is a test of the spelling checker";
    h.engine
        .test_set_scheduled(listed(BACKEND, "c1", &format!("<p>{text}</p>"), at));
    let label = crate::send_options::sending_label(at);
    assert!(
        bubble_shows(&h.app, &label, text),
        "the bubble shows the text"
    );
}

#[test]
fn delete_on_a_listed_bubble_removes_it() {
    let h = Harness::new();
    let _log = wire(&h);
    h.select(BACKEND);
    let at = teams_proto::time::now_ms() + 86_400_000;
    h.engine
        .test_set_scheduled(listed(BACKEND, "c1", "<p>later</p>", at));
    let label = crate::send_options::sending_label(at);
    assert_eq!(find_label(&h.app, &label).len(), 1);
    click(&button(&h.app, "Delete"));
    assert!(h.engine.test_scheduled().is_empty());
    assert!(find_label(&h.app, &label).is_empty(), "the bubble is gone");
    // The offline client may say the chat couldn't refresh; the delete itself raised nothing.
    assert!(
        !h.app.get_banner().to_lowercase().contains("scheduled"),
        "no scheduled-message error: {:?}",
        h.app.get_banner()
    );
}

/// Online with the offline client: every account call fails, as a refused request would.
fn go_online(h: &Harness) {
    h.engine
        .offline
        .store(false, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn a_failed_delete_keeps_the_bubble_and_says_so() {
    let h = Harness::new();
    let _log = wire(&h);
    h.select(BACKEND);
    let at = teams_proto::time::now_ms() + 86_400_000;
    h.engine
        .test_set_scheduled(listed(BACKEND, "c1", "<p>later</p>", at));
    go_online(&h);
    click(&button(&h.app, "Delete"));
    let label = crate::send_options::sending_label(at);
    assert_eq!(find_label(&h.app, &label).len(), 1, "still waiting");
    assert_eq!(h.engine.test_scheduled().len(), 1);
    let banner = h.app.get_banner().to_string();
    assert!(
        banner.starts_with("Couldn't delete the scheduled message"),
        "{banner}"
    );
}

#[test]
fn a_refused_schedule_shows_no_bubble_and_puts_the_text_back() {
    let h = Harness::new();
    let _log = wire(&h);
    h.select(BACKEND);
    let at = teams_proto::time::now_ms() + 86_400_000;
    ss(&h.app).set_schedule_at(at.to_string().into());
    ss(&h.app).set_schedule_label(crate::send_options::when_label(at).into());
    h.type_in_composer("see you tomorrow");
    go_online(&h);
    enter(&h.app);
    assert!(h.engine.test_scheduled().is_empty());
    let label = crate::send_options::sending_label(at);
    assert!(find_label(&h.app, &label).is_empty(), "no bubble");
    assert_eq!(h.app.get_draft(), "see you tomorrow");
    let banner = h.app.get_banner().to_string();
    assert!(
        banner.starts_with("Couldn't schedule the message"),
        "{banner}"
    );
}
// ---- The schedule picker: any time, quick choices, the month calendar ----

fn open_picker(app: &AppWindow) {
    click(&button(app, "Send options"));
    click(&button(app, "Schedule message"));
    assert!(!find_label(app, "Send this message on").is_empty());
}

fn time_field(app: &AppWindow) -> ElementHandle {
    ElementHandle::find_by_element_id(app, "SchedulePicker::time-input")
        .next()
        .expect("the time field")
}

/// What the time field shows.
fn time_value(app: &AppWindow) -> String {
    time_field(app).accessible_value().unwrap().to_string()
}

fn press(app: &AppWindow, k: Key) {
    key(app, k, true);
    key(app, k, false);
}

/// Clicks the time field, replaces what it holds with `text` and presses Enter.
fn type_time(app: &AppWindow, text: &str) {
    click(&time_field(app));
    for _ in 0..30 {
        press(app, Key::Backspace);
        press(app, Key::Delete);
    }
    type_text(app, text);
    enter(app);
}

/// An arrow key, with Shift held when `shift`.
fn arrow(app: &AppWindow, k: Key, shift: bool) {
    if shift {
        key(app, Key::Shift, true);
    }
    press(app, k);
    if shift {
        key(app, Key::Shift, false);
    }
}

#[test]
fn typing_8_34_pm_sets_the_minutes_to_1234() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    type_time(&app, "8:34 PM");
    assert_eq!(ss(&app).get_pick_minutes(), 1234);
    assert_eq!(time_value(&app), time_label(1234));
    assert!(!ss(&app).get_time_error());
    // Other ways to write a time, Arabic digits among them.
    type_time(&app, "835");
    assert_eq!(ss(&app).get_pick_minutes(), 8 * 60 + 35);
    type_time(&app, "20:35");
    assert_eq!(ss(&app).get_pick_minutes(), 1235);
    type_time(&app, "٨:٣٥ ص");
    assert_eq!(ss(&app).get_pick_minutes(), 8 * 60 + 35);
}

#[test]
fn a_time_that_is_not_one_shows_an_error_and_keeps_the_last_good_time() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    type_time(&app, "9:15 PM");
    assert_eq!(ss(&app).get_pick_minutes(), 21 * 60 + 15);
    type_time(&app, "25:99");
    assert_eq!(ss(&app).get_pick_minutes(), 21 * 60 + 15, "kept");
    assert!(ss(&app).get_time_error());
    assert!(!find_label(&app, "Not a time. Try 8:35 PM or 20:35.").is_empty());
    assert_eq!(button(&app, "Continue").accessible_enabled(), Some(false));
    // A good time takes the error away.
    type_time(&app, "10:05 PM");
    assert!(!ss(&app).get_time_error());
    assert_eq!(ss(&app).get_pick_minutes(), 22 * 60 + 5);
    assert!(find_label(&app, "Not a time. Try 8:35 PM or 20:35.").is_empty());
    assert_eq!(button(&app, "Continue").accessible_enabled(), Some(true));
}

#[test]
fn up_and_down_move_the_time_by_a_minute_and_shift_by_fifteen() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    assert_eq!(ss(&app).get_pick_minutes(), 480);
    click(&time_field(&app));
    arrow(&app, Key::UpArrow, false);
    assert_eq!(ss(&app).get_pick_minutes(), 481);
    arrow(&app, Key::UpArrow, true);
    assert_eq!(ss(&app).get_pick_minutes(), 496, "Shift+Up adds 15");
    arrow(&app, Key::DownArrow, true);
    arrow(&app, Key::DownArrow, false);
    assert_eq!(ss(&app).get_pick_minutes(), 480);
    assert_eq!(time_value(&app), time_label(480));
    // Around midnight: the day stays.
    ss(&app).set_pick_minutes(0);
    arrow(&app, Key::DownArrow, false);
    assert_eq!(ss(&app).get_pick_minutes(), 1439);
    assert_eq!(ss(&app).get_pick_day(), 1);
}

#[test]
fn the_hour_and_minute_steppers_move_by_one_each() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    click(&button(&app, "Later minute"));
    assert_eq!(ss(&app).get_pick_minutes(), 481);
    click(&button(&app, "Earlier minute"));
    click(&button(&app, "Later hour"));
    assert_eq!(ss(&app).get_pick_minutes(), 540);
    click(&button(&app, "Earlier hour"));
    click(&button(&app, "Earlier hour"));
    assert_eq!(ss(&app).get_pick_minutes(), 420);
    assert_eq!(time_value(&app), time_label(420));
    // Text that is not a time is dropped by a step.
    click(&time_field(&app));
    type_text(&app, "x");
    click(&button(&app, "Later minute"));
    assert_eq!(ss(&app).get_pick_minutes(), 421);
    assert_eq!(time_value(&app), time_label(421));
}

#[test]
fn a_quick_choice_fills_the_day_and_the_time() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    ss(&app).set_pick_day(5);
    ss(&app).set_pick_minutes(0);
    click(&button(&app, &format!("Tomorrow {}", time_label(9 * 60))));
    assert_eq!(ss(&app).get_pick_day(), 1);
    assert_eq!(ss(&app).get_pick_minutes(), 9 * 60);
    assert_eq!(time_value(&app), time_label(9 * 60));
    assert_eq!(button(&app, "Continue").accessible_enabled(), Some(true));

    // "In 1 hour" is an hour from now, in whole minutes.
    let before = Local::now();
    click(&button(&app, "In 1 hour"));
    let (day, minutes) = (ss(&app).get_pick_day(), ss(&app).get_pick_minutes());
    let at = resolve(day, minutes, before).expect("in the future");
    let wait = (at - before.timestamp_millis()) / 60_000;
    assert!((59..=61).contains(&wait), "{wait} minutes ahead");

    // The Monday choice is the next Monday (not offered on a Sunday).
    let to_monday = 7 - Local::now().weekday().num_days_from_monday() as i32;
    let monday = buttons(&app, &format!("Monday {}", time_label(9 * 60)))
        .into_iter()
        .next();
    if to_monday == 1 {
        assert!(monday.is_none());
    } else if to_monday < 7 {
        click(&monday.expect("the Monday choice"));
        assert_eq!(ss(&app).get_pick_day(), to_monday);
        assert_eq!(ss(&app).get_pick_minutes(), 9 * 60);
    }
}

#[test]
fn a_time_today_that_has_passed_disables_continue() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    ss(&app).set_pick_day(0);
    type_time(&app, "12:00 AM");
    assert_eq!(ss(&app).get_pick_minutes(), 0);
    assert!(!find_label(&app, "Pick a time in the future").is_empty());
    let cont = button(&app, "Continue");
    assert_eq!(cont.accessible_enabled(), Some(false));
    click(&cont);
    assert_eq!(ss(&app).get_schedule_at(), "");
    assert!(
        !find_label(&app, "Send this message on").is_empty(),
        "still open"
    );
    // The same time tomorrow is fine.
    click(&button(&app, "Later day"));
    assert!(find_label(&app, "Pick a time in the future").is_empty());
    assert_eq!(button(&app, "Continue").accessible_enabled(), Some(true));
}

#[test]
fn the_summary_line_reads_the_full_day_and_time() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    type_time(&app, "8:35 PM");
    let ms = resolve(1, 20 * 60 + 35, Local::now()).unwrap();
    let summary = format!("Sends {}", when_label(ms));
    assert!(summary.ends_with(&format!(" at {}", time_label(20 * 60 + 35))));
    assert!(summary.contains(&day_label(1, Local::now())));
    assert_eq!(find_label(&app, &summary).len(), 1, "{summary}");
    click(&button(&app, "Continue"));
    assert_eq!(ss(&app).get_schedule_at(), ms.to_string());
    assert_eq!(ss(&app).get_schedule_label(), when_label(ms));
}

#[test]
fn pressing_enter_on_a_taken_time_continues() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    type_time(&app, "8:35 PM");
    assert_eq!(
        ss(&app).get_schedule_at(),
        "",
        "the first Enter takes the time"
    );
    enter(&app);
    assert_ne!(ss(&app).get_schedule_at(), "");
}

#[test]
fn clicking_continue_takes_a_typed_time_first() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    click(&time_field(&app));
    type_text(&app, "9:41 PM");
    click(&button(&app, "Continue"));
    let label = ss(&app).get_schedule_label().to_string();
    let want = format!(" at {}", time_label(21 * 60 + 41));
    assert!(label.ends_with(&want), "{label}");
}

#[test]
fn the_date_label_opens_a_month_calendar_for_today_through_sixty_days() {
    let app = app_with_messages(vec![]);
    open_picker(&app);
    let now = Local::now();
    click(&button(&app, &day_label(1, now)));
    assert!(ss(&app).get_cal_open());
    assert!(!find_label(&app, &month_title(0, now)).is_empty());
    assert!(
        buttons(&app, "Previous month").is_empty(),
        "not before this month"
    );
    // Past days cannot be picked.
    if let Some(yesterday) = buttons(&app, &day_label(-1, now)).into_iter().next() {
        assert_eq!(yesterday.accessible_enabled(), Some(false));
        click(&yesterday);
        assert!(ss(&app).get_cal_open());
        assert_eq!(ss(&app).get_pick_day(), 1);
    }
    // The next months are there up to the one with day 60.
    let mut months = 0;
    while let Some(next) = buttons(&app, "Next month").into_iter().next() {
        click(&next);
        months += 1;
        assert!(months < 6);
    }
    assert_eq!(
        months,
        crate::send_options::month_of_day(crate::send_options::MAX_DAYS, now)
    );
    // The last allowed day is on, the one after it is off.
    let last = buttons(&app, &day_label(60, now));
    assert_eq!(last.last().unwrap().accessible_enabled(), Some(true));
    if let Some(after) = buttons(&app, &day_label(61, now)).into_iter().next() {
        assert_eq!(after.accessible_enabled(), Some(false));
    }
    click(&button(&app, "Back"));
    assert!(!ss(&app).get_cal_open());
    assert_eq!(ss(&app).get_pick_day(), 1);

    // Pick a day: three days from now.
    click(&button(&app, &day_label(1, now)));
    click(&button(&app, &day_label(3, now)));
    assert_eq!(ss(&app).get_pick_day(), 3);
    assert!(!ss(&app).get_cal_open(), "the calendar closes");
}

#[test]
fn the_picker_fits_a_760_by_600_window_and_its_text_stays_inside() {
    let app = app_with_messages(vec![]);
    app.window().set_size(LogicalSize::new(760., 600.));
    open_picker(&app);
    let inside_window = |e: &ElementHandle, what: &str| {
        let (p, s) = (e.absolute_position(), e.size());
        assert!(
            p.x >= 0. && p.y >= 0. && p.x + s.width <= 760. && p.y + s.height <= 600.,
            "{what} sticks out of the window: {p:?} {s:?}"
        );
    };
    let picker = |app: &AppWindow| {
        ElementHandle::find_by_accessible_label(app, "Send this message on")
            .find(|e| e.accessible_role() == Some(AccessibleRole::Groupbox))
            .expect("the picker")
    };
    let p = picker(&app);
    inside_window(&p, "the picker");
    super::overflow::assert_texts_inside(&p, "the picker");
    for name in [
        "Cancel",
        "Continue",
        "Earlier day",
        "Later day",
        "Later hour",
        "Later minute",
    ] {
        inside_window(&button(&app, name), name);
    }
    inside_window(&time_field(&app), "the time field");
    for q in ss(&app).get_quick().iter() {
        inside_window(&button(&app, &q.label), &q.label);
    }
    // The calendar view, too.
    click(&button(&app, &day_label(1, Local::now())));
    let p = picker(&app);
    inside_window(&p, "the picker with the calendar");
    super::overflow::assert_texts_inside(&p, "the calendar");
    inside_window(&button(&app, "Back"), "Back");
}
