//! The send button's options, driven through the real window: right-click Send or its chevron
//! for "Schedule message" and "Set delivery options" (Standard, Important, Urgent); the picked
//! send time's banner; the scheduled "Sending on …" bubbles with Edit and Delete. The engine is
//! offline: a schedule is kept here only (the account call is skipped).

use super::*;
use crate::SendStore;
use crate::backend::ScheduledCmd;
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
    assert!(!find_label(&h.app, "8:00 AM").is_empty());
    // Later by half an hour, then back.
    click(&button(&h.app, "Later time"));
    assert!(!find_label(&h.app, "8:30 AM").is_empty());
    click(&button(&h.app, "Earlier time"));
    click(&button(&h.app, "Continue"));
    let at = ss(&h.app).get_schedule_at().to_string();
    let at_ms: i64 = at.parse().expect("a time");
    assert!(at_ms > teams_proto::time::now_ms());
    let label = ss(&h.app).get_schedule_label().to_string();
    assert!(label.ends_with(" at 8:00 AM"), "{label}");
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
    assert_eq!(h.app.get_banner(), "");
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
