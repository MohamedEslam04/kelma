//! Call history: parsing the `48:calllogs` thread and paging it (offline, synthetic fixtures
//! built from the shape the live service returns).

use calling::history::{
    CallDirection, CallOutcome, CallRecord, call_log_url, collect_history, parse_page,
};
use serde_json::{Value, json};
use std::cell::RefCell;

const ME: &str = "8:orgid:00000000-0000-0000-0000-00000000000a";
const ADA: &str = "8:orgid:00000000-0000-0000-0000-00000000000b";
const BOB: &str = "8:orgid:00000000-0000-0000-0000-00000000000c";
const CY: &str = "8:orgid:00000000-0000-0000-0000-00000000000d";

fn fixture(name: &str) -> Value {
    let text = match name {
        "page1" => include_str!("fixtures/history/page1.json"),
        "page2" => include_str!("fixtures/history/page2.json"),
        other => panic!("no fixture {other}"),
    };
    serde_json::from_str(text).unwrap()
}

fn page1() -> Vec<CallRecord> {
    parse_page(&fixture("page1")).records
}

fn by_id(records: &[CallRecord], n: u32) -> &CallRecord {
    let id = format!("00000000-0000-0000-0000-0000000002{n:02}");
    records.iter().find(|r| r.id == id).expect("record")
}

#[test]
fn an_accepted_incoming_call_has_the_caller_a_duration_and_an_end() {
    let records = page1();
    let r = by_id(&records, 1);
    assert_eq!(r.direction, CallDirection::Incoming);
    assert_eq!(r.outcome, CallOutcome::Accepted);
    assert_eq!(r.other_mri.as_deref(), Some(ADA));
    assert_eq!(r.other_name.as_deref(), Some("Ada Lovelace"));
    // connect 10:51:41.464 to end 12:50:31.263
    assert_eq!(r.duration_secs, Some(7129));
    assert_eq!(r.start_ms, 1_794_912_696_849);
    assert_eq!(r.end_ms, Some(1_794_919_831_263));
    assert!(!r.is_group);
    assert!(!r.is_missed());
}

#[test]
fn an_outgoing_call_names_the_target_but_the_log_has_no_name_for_them() {
    let records = page1();
    let r = by_id(&records, 2);
    assert_eq!(r.direction, CallDirection::Outgoing);
    assert_eq!(r.other_mri.as_deref(), Some(ADA));
    assert_eq!(r.other_name, None);
    assert_eq!(r.duration_secs, Some(252));
}

#[test]
fn missed_declined_and_unanswered_calls_have_no_duration() {
    let records = page1();
    let missed = by_id(&records, 3);
    assert_eq!(missed.outcome, CallOutcome::Missed);
    assert!(missed.is_missed());
    assert_eq!(missed.duration_secs, None);
    assert_eq!(missed.other_name.as_deref(), Some("Bob Builder"));
    assert_eq!(missed.other_mri.as_deref(), Some(BOB));

    // An unanswered outgoing call is "missed" by outcome but not a missed call for the user.
    let unanswered = by_id(&records, 4);
    assert_eq!(unanswered.direction, CallDirection::Outgoing);
    assert_eq!(unanswered.outcome, CallOutcome::Missed);
    assert!(!unanswered.is_missed());

    let declined = by_id(&records, 5);
    assert_eq!(declined.outcome, CallOutcome::Declined);
    assert_eq!(declined.other_mri.as_deref(), Some(CY));
    assert!(!declined.is_missed());
}

#[test]
fn a_multi_party_call_is_a_group_with_its_thread_and_the_other_participants() {
    let records = page1();
    let r = by_id(&records, 6);
    assert!(r.is_group);
    assert_eq!(
        r.thread_id.as_deref(),
        Some("19:meeting_synthetic@thread.v2")
    );
    assert_eq!(r.duration_secs, Some(2280));
    // Everyone but the user, who is the target of an incoming call.
    assert_eq!(r.participants, vec![BOB, ADA, CY]);
    assert!(!r.participants.iter().any(|p| p == ME));
}

#[test]
fn calling_into_a_voicemail_box_is_a_voicemail_call_to_the_callee() {
    let records = page1();
    let r = by_id(&records, 7);
    assert_eq!(r.direction, CallDirection::Outgoing);
    assert_eq!(r.outcome, CallOutcome::Voicemail);
    assert_eq!(r.other_mri.as_deref(), Some(ADA));
    assert_eq!(r.duration_secs, Some(35));
    assert!(r.is_voicemail());
    assert!(!r.is_missed());
}

#[test]
fn a_voicemail_left_for_the_user_is_the_sparse_entry_with_a_mail_id() {
    let records = page1();
    let r = by_id(&records, 10);
    assert_eq!(r.direction, CallDirection::Incoming);
    assert_eq!(r.outcome, CallOutcome::Voicemail);
    assert_eq!(r.other_mri.as_deref(), Some(ADA));
    assert_eq!(r.other_name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(r.duration_secs, None);
    assert_eq!(
        r.voicemail_message_id.as_deref(),
        Some("<synthetic-voicemail@example.invalid>")
    );
    assert!(!r.is_missed());
    assert!(!r.is_group);
}

#[test]
fn an_outgoing_group_call_has_no_target_in_the_log() {
    let body = json!({"messages": [{"id": "m", "properties": {"call-log": {
        "callId": "g1", "callDirection": "outgoing", "callState": "accepted",
        "callType": "multiParty", "startTime": "2026-11-17T10:00:00Z",
        "connectTime": "2026-11-17T10:00:02Z", "endTime": "2026-11-17T10:10:02Z",
        "originator": ME, "target": null, "targetParticipant": null,
        "originatorParticipant": {"id": ME, "displayName": "Me Myself", "type": "default"},
        "participants": [ME, ADA, BOB],
    }}}]});
    let r = &parse_page(&body).records[0];
    assert!(r.is_group);
    assert_eq!(r.other_mri, None);
    assert_eq!(r.participants, vec![ADA, BOB]);
    assert_eq!(r.duration_secs, Some(600));
}

#[test]
fn a_page_is_newest_first_and_skips_other_messages() {
    let records = page1();
    // 8 call logs; the recording message and the unparsable log are dropped.
    assert_eq!(records.len(), 8);
    assert!(records.windows(2).all(|w| w[0].start_ms >= w[1].start_ms));
}

#[test]
fn the_call_log_may_be_an_object_and_unknown_fields_are_ignored() {
    // Page 2 carries record 8 with `call-log` as a JSON object, not a string.
    let page = parse_page(&fixture("page2"));
    assert_eq!(by_id(&page.records, 8).other_mri.as_deref(), Some(CY));
    assert_eq!(page.older, None);

    let body = json!({"messages": [{
        "id": "m1",
        "properties": {"call-log": {
            "callId": "c1", "callDirection": "incoming", "callState": "accepted",
            "startTime": "2026-11-17T10:00:00Z", "connectTime": "2026-11-17T10:00:01Z",
            "endTime": "2026-11-17T10:01:01Z", "futureField": [1, 2],
            "originator": ADA, "target": ME,
            "originatorParticipant": {"id": ADA, "displayName": "Ada", "mystery": true},
        }},
    }]});
    let records = parse_page(&body).records;
    assert_eq!(records[0].duration_secs, Some(60));
    assert_eq!(records[0].other_name.as_deref(), Some("Ada"));
}

#[test]
fn an_unknown_or_missing_call_state_is_judged_by_the_connect_time() {
    let mk = |connect: Value, state: Value| {
        let mut log = json!({
            "callId": "c1", "callDirection": "incoming",
            "startTime": "2026-11-17T10:00:00Z", "connectTime": connect,
            "endTime": "2026-11-17T10:01:00Z", "originator": ADA, "target": ME,
        });
        log["callState"] = state;
        parse_page(&json!({"messages": [{"id": "m", "properties": {"call-log": log.to_string()}}]}))
            .records
    };
    assert_eq!(
        mk(json!("2026-11-17T10:00:05Z"), json!("somethingNew"))[0].outcome,
        CallOutcome::Accepted
    );
    assert_eq!(mk(Value::Null, Value::Null)[0].outcome, CallOutcome::Missed);
}

#[test]
fn an_entry_without_a_direction_or_start_is_skipped() {
    let body = json!({"messages": [
        {"id": "a", "properties": {"call-log": {"callId": "a", "startTime": "2026-11-17T10:00:00Z"}}},
        {"id": "b", "properties": {"call-log": {"callId": "b", "callDirection": "incoming"}}},
    ]});
    assert!(parse_page(&body).records.is_empty());
}

#[test]
fn the_older_page_link_is_read_from_the_metadata() {
    let page = parse_page(&fixture("page1"));
    assert!(page.older.as_deref().unwrap().contains("syncState=abc"));
    assert_eq!(parse_page(&json!({"messages": []})).older, None);
}

#[test]
fn the_call_log_url_reads_the_calllogs_thread() {
    assert_eq!(
        call_log_url("https://emea.ng.msg.teams.example.invalid"),
        "https://emea.ng.msg.teams.example.invalid/v1/users/ME/conversations/48%3Acalllogs/messages?view=msnp24Equivalent&pageSize=200"
    );
}

/// Serves page1, then page2 (what `backwardLink` of page1 points at), recording the URLs asked.
async fn history(limit: usize) -> (Vec<CallRecord>, Vec<String>) {
    let asked = RefCell::new(Vec::new());
    let records = collect_history("first".into(), limit, |url| {
        asked.borrow_mut().push(url.clone());
        let body = if url == "first" {
            fixture("page1")
        } else {
            fixture("page2")
        };
        async move { Ok(body) }
    })
    .await
    .unwrap();
    (records, asked.into_inner())
}

#[tokio::test]
async fn paging_follows_the_backward_link_and_drops_duplicates() {
    let (records, asked) = history(100).await;
    assert_eq!(asked.len(), 2);
    assert!(asked[1].contains("syncState=abc"));
    // 8 + 2 new (record 3 repeats across the pages).
    assert_eq!(records.len(), 10);
    assert!(records.windows(2).all(|w| w[0].start_ms >= w[1].start_ms));
    let mut ids: Vec<_> = records.iter().map(|r| r.id.clone()).collect();
    ids.dedup();
    assert_eq!(ids.len(), 10);
}

#[tokio::test]
async fn paging_stops_once_the_limit_is_reached_and_truncates() {
    let (records, asked) = history(5).await;
    assert_eq!(asked.len(), 1, "no second request when page one has enough");
    assert_eq!(records.len(), 5);
    assert_eq!(records[0].id, by_id(&page1(), 1).id);
}

#[tokio::test]
async fn paging_stops_on_a_link_that_leads_nowhere_new() {
    // Every page links to itself: the loop must end instead of spinning.
    let calls = RefCell::new(0);
    let records = collect_history("first".into(), 100, |_| {
        *calls.borrow_mut() += 1;
        let body = fixture("page1");
        async move { Ok(body) }
    })
    .await
    .unwrap();
    assert_eq!(records.len(), 8);
    assert_eq!(*calls.borrow(), 2);
}

#[tokio::test]
async fn a_failing_first_page_is_an_error_but_a_later_one_keeps_what_we_have() {
    let err = collect_history("first".into(), 10, |_| async {
        Err(calling::Error::Protocol("boom".into()))
    })
    .await;
    assert!(err.is_err());

    let n = RefCell::new(0);
    let records = collect_history("first".into(), 100, |_| {
        *n.borrow_mut() += 1;
        let first = *n.borrow() == 1;
        async move {
            if first {
                Ok(fixture("page1"))
            } else {
                Err(calling::Error::Protocol("boom".into()))
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(records.len(), 8);
}
