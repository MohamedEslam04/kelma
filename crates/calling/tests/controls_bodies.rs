//! In-meeting controls: self mute/unmute (`updateEndpointState`) and raise/lower hand
//! (`publishState` / `removeState`), as pure request builders (offline, synthetic fixtures).

use calling::Links;
use calling::signaling::bodies::parse_conversation;
use calling::signaling::controls::{
    Controls, HandReceipt, control_url, parse_hand_receipt, raise_hand_body, remove_state_body,
    update_endpoint_state_body,
};
use calling::signaling::{CallSession, CallbackLinks, LocalEndpoint};
use serde_json::{Value, json};

const CONTROLLER: &str = "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAAAAAAAAAAAAAAAAAAAA?i=1";
const ME: &str = "8:orgid:00000000-0000-0000-0000-0000000000b2";

fn session() -> CallSession {
    CallSession {
        me: LocalEndpoint {
            mri: ME.into(),
            display_name: "Self Example".into(),
            endpoint_id: "sess-1".into(),
            participant_id: "part-1".into(),
            language_id: "en-US".into(),
        },
        callbacks: CallbackLinks::new("https://trouter.example.invalid/v4/f/AbCd/", "sess-1"),
        chain_id: "chain-1".into(),
        media_leg_id: "LEG1".into(),
    }
}

fn fixture(name: &str) -> Value {
    let text = match name {
        "join_answer" => include_str!("fixtures/controls/join_answer.json"),
        "publish_state_response" => include_str!("fixtures/controls/publish_state_response.json"),
        other => panic!("no fixture {other}"),
    };
    serde_json::from_str(text).unwrap()
}

fn joined_links() -> Links {
    parse_conversation(&fixture("join_answer"), None)
        .unwrap()
        .links
}

#[test]
fn self_mute_publishes_the_endpoint_state_with_our_identity() {
    let s = session();
    let b = update_endpoint_state_body(&s, 3, true);
    assert_eq!(b["from"], s.me.wire());
    assert_eq!(b["endpointState"]["endpointStateSequenceNumber"], 3);
    assert_eq!(b["endpointState"]["state"], json!({"isMuted": true}));
    assert_eq!(
        b["endpointState"]["endpointProperties"],
        json!({"additionalEndpointProperties": {"infoShownInReportMode": "FullInformation"}})
    );
    assert_eq!(
        update_endpoint_state_body(&s, 4, false)["endpointState"]["state"]["isMuted"],
        false
    );
}

#[test]
fn raising_publishes_a_user_level_raise_hands_state() {
    let s = session();
    let b = raise_hand_body(&s, 1);
    assert_eq!(b["from"], s.me.wire());
    assert_eq!(
        b["publishedState"],
        json!({"stateType": "raiseHands", "level": "user", "content": {"skinTone": 0}, "sequenceNumber": 1})
    );
}

#[test]
fn lowering_removes_the_named_states() {
    let s = session();
    let b = remove_state_body(&s, 2, &["hand-b2".to_owned()]);
    assert_eq!(b["from"], s.me.wire());
    assert_eq!(b["sequenceNumber"], 2);
    assert_eq!(b["scope"], "specified");
    assert_eq!(b["stateIds"], json!(["hand-b2"]));
}

#[test]
fn the_publish_answer_names_the_hand_state_and_its_queue_rank() {
    assert_eq!(
        parse_hand_receipt(&fixture("publish_state_response")),
        HandReceipt {
            state_id: Some("hand-b2".into()),
            rank: Some(2)
        }
    );
    assert_eq!(
        parse_hand_receipt(&json!({})),
        HandReceipt {
            state_id: None,
            rank: None
        }
    );
}

#[test]
fn control_urls_prefer_the_conversation_link_and_fall_back_to_the_controller() {
    let links = joined_links();
    assert_eq!(
        control_url(&links, CONTROLLER, "updateEndpointState"),
        "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAAAAAAAAAAAAAAAAAAAA/updateEndpointState?i=1"
    );
    assert_eq!(
        control_url(&Links::default(), CONTROLLER, "publishState"),
        "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAAAAAAAAAAAAAAAAAAAA/publishState?i=1"
    );
}

#[test]
fn each_mute_change_bumps_the_endpoint_sequence_and_remembers_the_state() {
    let (s, links) = (session(), joined_links());
    let mut c = Controls::default();
    assert!(!c.muted());
    let (url, b) = c.mute_request(&s, &links, CONTROLLER, true);
    assert!(url.contains("/updateEndpointState?"));
    // The join announced sequence 0, so the first update is 1.
    assert_eq!(b["endpointState"]["endpointStateSequenceNumber"], 1);
    assert_eq!(b["endpointState"]["state"]["isMuted"], true);
    assert!(c.muted());
    let (_, b) = c.mute_request(&s, &links, CONTROLLER, false);
    assert_eq!(b["endpointState"]["endpointStateSequenceNumber"], 2);
    assert!(!c.muted());
    // Repeating the current state (after admission) still bumps the sequence.
    let (_, b) = c.repeat_mute_request(&s, &links, CONTROLLER);
    assert_eq!(b["endpointState"]["endpointStateSequenceNumber"], 3);
    assert_eq!(b["endpointState"]["state"]["isMuted"], false);
}

#[test]
fn a_raised_hand_is_lowered_by_the_state_id_the_service_returned() {
    let (s, links) = (session(), joined_links());
    let mut c = Controls::default();
    let (url, b) = c.raise_hand_request(&s, &links, CONTROLLER);
    assert!(url.contains("/publishState?"));
    assert_eq!(b["publishedState"]["sequenceNumber"], 1);
    c.hand_raised(&parse_hand_receipt(&fixture("publish_state_response")));
    assert_eq!(c.hand_state_id(), Some("hand-b2"));

    let (url, b) = c
        .lower_hand_request(&s, &links, CONTROLLER, None)
        .expect("a raised hand can be lowered");
    assert!(url.contains("/removeState?"));
    assert_eq!(b["sequenceNumber"], 2);
    assert_eq!(b["stateIds"], json!(["hand-b2"]));
    assert_eq!(c.hand_state_id(), None);
}

#[test]
fn lowering_falls_back_to_the_rosters_hand_and_is_a_no_op_without_one() {
    let (s, links) = (session(), joined_links());
    let mut c = Controls::default();
    assert!(c.lower_hand_request(&s, &links, CONTROLLER, None).is_none());
    let (_, b) = c
        .lower_hand_request(&s, &links, CONTROLLER, Some("hand-from-roster"))
        .unwrap();
    assert_eq!(b["stateIds"], json!(["hand-from-roster"]));
}
