//! 1:1 call operations: add a person (`addParticipant`), hold and resume (a same-peer
//! `mediaRenegotiation`), and blind transfer (`transfer`), as pure request builders (offline,
//! synthetic values only).

use calling::Links;
use calling::signaling::bodies::{negotiation_body, parse_attach, renegotiation_body};
use calling::signaling::call_ops::{
    TransferOutcome, add_participant_body, add_participant_request, hold_offer, keep_media_running,
    parse_transfer_completion, transfer_body, transfer_request,
};
use calling::signaling::{CallSession, CallbackLinks, LocalEndpoint};
use calling::{Modality, SdpBlob};
use serde_json::json;

const CONTROLLER: &str =
    "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAA?i=1";
const ME: &str = "8:orgid:00000000-0000-0000-0000-0000000000b2";
const ADA: &str = "8:orgid:00000000-0000-0000-0000-0000000000a1";

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

fn links(pairs: &[(&str, &str)]) -> Links {
    Links(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    )
}

// --- add a person --------------------------------------------------------------------------

#[test]
fn adding_a_person_names_them_with_a_fresh_participant_id_and_both_callbacks() {
    let s = session();
    let b = add_participant_body(&s, ADA, "Ada Example");
    assert_eq!(b["participants"]["from"], s.me.wire());
    let to = b["participants"]["to"].as_array().unwrap();
    assert_eq!(to.len(), 1);
    assert_eq!(to[0]["id"], ADA);
    assert_eq!(to[0]["displayName"], "Ada Example");
    let pid = to[0]["participantId"].as_str().unwrap();
    assert_eq!(pid.len(), 36, "a uuid");
    assert_ne!(
        add_participant_body(&s, ADA, "Ada Example")["participants"]["to"][0]["participantId"],
        pid,
        "fresh per request"
    );
    assert_eq!(b["disableUnmute"], false);
    assert_eq!(b["participantInvitationData"], json!({}));
    assert!(b["replacementDetails"].is_null());
    // Without a group chat thread the web client sends `debugContent: {}` (mautrix-teams).
    assert_eq!(b["debugContent"], json!({}));
    assert!(b.get("groupChat").is_none() && b.get("callInvitation").is_none());
    assert_eq!(
        b["links"]["addParticipantSuccess"],
        s.callbacks.conversation("addParticipantSuccess")
    );
    assert_eq!(
        b["links"]["addParticipantFailure"],
        s.callbacks.conversation("addParticipantFailure")
    );
}

#[test]
fn adding_goes_to_the_add_participant_link_else_under_the_controller() {
    let s = session();
    let (url, body) = add_participant_request(
        &s,
        &links(&[(
            "addParticipant",
            "https://cc.example/conv/AAAA/addParticipant",
        )]),
        CONTROLLER,
        ADA,
        "Ada Example",
    );
    assert_eq!(url, "https://cc.example/conv/AAAA/addParticipant");
    assert_eq!(body["participants"]["to"][0]["id"], ADA);
    let (url, _) = add_participant_request(&s, &Links::default(), CONTROLLER, ADA, "Ada");
    assert_eq!(
        url,
        "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAA/addParticipant?i=1"
    );
}

#[test]
fn the_attach_answer_keeps_the_conversations_add_links() {
    let a = parse_attach(&json!({
        "callInvitation": {"links": {"acceptance": "https://cc.example/accept"}},
        "additionalActionResponses": [{"output": {
            "conversationController": "https://cc.example/conv/1",
            "links": {
                "addParticipant": "https://cc.example/conv/1/addParticipant",
                "addParticipantAndModality": "https://cc.example/conv/1/add",
                "leave": "https://cc.example/conv/1/leave",
            },
        }}],
    }))
    .unwrap();
    assert_eq!(
        a.links.get("addParticipant"),
        Some("https://cc.example/conv/1/addParticipant")
    );
    assert_eq!(
        a.links.get("addParticipantAndModality"),
        Some("https://cc.example/conv/1/add")
    );
    // A 1:1 call keeps hanging up on its leg, not the conversation.
    assert_eq!(a.links.get("leave"), None);
}

// --- hold and resume -------------------------------------------------------------------------

const OFFER: &str = "v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=sendrecv\r\n\
m=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:1\r\na=recvonly\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=mid:2\r\n";

#[test]
fn a_hold_offer_puts_every_media_line_inactive_and_nothing_else() {
    let held = hold_offer(OFFER);
    assert_eq!(
        held,
        OFFER
            .replace("a=sendrecv", "a=inactive")
            .replace("a=recvonly", "a=inactive")
    );
    assert!(held.contains("\r\n"), "line endings kept");
}

#[test]
fn a_hold_answer_mirrors_our_offer_so_nothing_stops_locally() {
    let answer = "v=0\r\ns=-\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=inactive\r\n\
m=video 0 UDP/TLS/RTP/SAVPF 96\r\na=mid:1\r\na=inactive\r\n";
    let kept = keep_media_running(answer, OFFER);
    // Our offer sent and received audio (the answer mirrors it) and only received video.
    assert!(kept.contains("a=mid:0\r\na=sendrecv\r\n"));
    assert!(kept.contains("a=mid:1\r\na=sendonly\r\n"));
    assert!(!kept.contains("inactive"));
}

#[test]
fn a_same_peer_offer_is_not_a_new_offer() {
    let s = session();
    let offer = SdpBlob::ngc("v=0\r\n");
    let b = renegotiation_body(&s, &[Modality::Audio], &offer, false);
    let n = &b["mediaNegotiation"];
    assert_eq!(n["callModalities"], json!(["Audio"]));
    assert_eq!(n["sender"], s.me.wire());
    assert_eq!(n["links"]["mediaAnswer"], s.callbacks.call("mediaAnswer"));
    assert_eq!(n["links"]["rejection"], s.callbacks.call("rejection"));
    assert_eq!(n["mediaContent"]["newOffer"], false);
    assert_eq!(n["mediaContent"]["blob"], "v=0\r\n");
    assert_eq!(n["mediaContent"]["mediaLegId"], "LEG1");
    assert_eq!(n["mediaContent"]["requiredFeatures"], "nonByPass");
    // The offer for a ring without one stays a new offer.
    assert_eq!(
        negotiation_body(&s, &[Modality::Audio], &offer)["mediaNegotiation"]["mediaContent"]["newOffer"],
        true
    );
}

// --- transfer --------------------------------------------------------------------------------

#[test]
fn a_blind_transfer_names_the_target_and_us_as_transferor() {
    let s = session();
    let b = transfer_body(&s, ADA);
    let t = &b["callTransfer"];
    assert_eq!(t["target"], json!({"id": ADA}));
    assert_eq!(
        t["transferor"]["details"],
        json!({"id": ME, "endpointId": "sess-1", "participantId": "part-1", "languageId": "en-US"})
    );
    assert!(t["transferor"]["authorizationToken"].is_null());
    assert!(t["replacementDetails"].is_null());
    assert!(t["transferContext"].is_null());
    assert_eq!(t["disableForwardingAndUnanswered"], false);
    assert_eq!(
        t["links"]["transferAcceptance"],
        s.callbacks.call("transferAcceptance")
    );
    assert_eq!(
        t["links"]["transferCompletion"],
        s.callbacks.call("transferCompletion")
    );
}

#[test]
fn a_transfer_goes_to_the_legs_transfer_link_only() {
    let s = session();
    let (url, body) = transfer_request(
        &s,
        &links(&[("transfer", "https://cc.example/leg/transfer")]),
        ADA,
    )
    .unwrap();
    assert_eq!(url, "https://cc.example/leg/transfer");
    assert_eq!(body["callTransfer"]["target"]["id"], ADA);
    assert!(transfer_request(&s, &Links::default(), ADA).is_none());
}

#[test]
fn the_transfer_completion_tells_success_from_failure() {
    assert_eq!(
        parse_transfer_completion(&json!({"transferCompletion": {"code": 0, "subCode": 0}})),
        Some(TransferOutcome::Done)
    );
    assert_eq!(
        parse_transfer_completion(&json!({"transferCompletion": {"code": 202}})),
        Some(TransferOutcome::Done)
    );
    assert_eq!(
        parse_transfer_completion(&json!({"transferCompletion": {"code": 603, "subCode": 10603}})),
        Some(TransferOutcome::Failed {
            code: 603,
            sub_code: 10603
        })
    );
    assert_eq!(parse_transfer_completion(&json!({})), None);
}
