//! Answering a ring with or without the caller's offer (offline, synthetic fixtures with the
//! shape of the live 2026-11-19 capture and fake values).

use calling::incoming::{Answering, NoOffer, OfferVia, Step};
use calling::invitation::{CallEvent, EndReason, NgcPush, classify, parse_call_event};
use calling::signaling::bodies::{
    Attached, attach_body, attach_offer_body, mediafree_acceptance_body, negotiation_body,
    offer_acceptance_body, parse_acceptance_answer, parse_attach, parse_inline_answer,
};
use calling::signaling::{CallSession, CallbackLinks, LocalEndpoint};
use calling::{CallInvitation, Links, Modality, SdpBlob};
use serde_json::{Value, json};

const SURL: &str = "https://trouter.example.invalid/v4/f/AbCd/";

fn session() -> CallSession {
    CallSession {
        me: LocalEndpoint {
            mri: "8:orgid:00000000-0000-0000-0000-0000000000b2".into(),
            display_name: "Me Example".into(),
            endpoint_id: "sess-1".into(),
            participant_id: "part-1".into(),
            language_id: "en-US".into(),
        },
        callbacks: CallbackLinks::new(SURL, "sess-1"),
        chain_id: "chain-1".into(),
        media_leg_id: "LEG1".into(),
    }
}

fn sdp(s: &str) -> SdpBlob {
    SdpBlob::ngc(s)
}

fn no_offer_push() -> (String, Value) {
    let v: Value = serde_json::from_str(include_str!(
        "fixtures/signaling/call_invitation_no_offer.json"
    ))
    .unwrap();
    (v["url"].as_str().unwrap().to_owned(), v["body"].clone())
}

fn no_offer_invitation() -> CallInvitation {
    let (url, body) = no_offer_push();
    match classify(&url, &body) {
        NgcPush::Invitation(inv) => *inv,
        other => panic!("not an invitation: {other:?}"),
    }
}

fn with_offer(mut inv: CallInvitation, acceptance: bool) -> CallInvitation {
    inv.offer = Some(sdp("v=0\r\nring-offer\r\n"));
    if acceptance {
        inv.links
            .0
            .insert("acceptance".into(), "https://cc.example/accept".into());
    }
    inv
}

fn attach_answer() -> Attached {
    parse_attach(
        &serde_json::from_str(include_str!(
            "fixtures/signaling/attach_response_with_offer.json"
        ))
        .unwrap(),
    )
    .unwrap()
}

/// Where the attach answer fixture says our offer goes.
fn new_offer_via() -> OfferVia {
    OfferVia::NewOffer(attach_answer().links.get("newOffer").unwrap().to_owned())
}

fn attached_without_offer() -> Attached {
    Attached {
        offer: None,
        media_leg_id: None,
        ..attach_answer()
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

fn media_answer_event() -> CallEvent {
    let body = json!({"mediaAnswer": {
        "mediaContent": {"contentType": "application/sdp-ngc-1.0", "blob": "v=0\r\ncaller-answer\r\n"},
        "links": {"mediaAcknowledgement": "https://cc.example/ack"},
    }});
    let url = session().callbacks.call("mediaAnswer");
    parse_call_event(&url, &body).unwrap().1
}

#[test]
fn a_ring_without_an_offer_is_still_an_invitation() {
    let inv = no_offer_invitation();
    assert_eq!(inv.offer, None);
    assert_eq!(inv.call_id, "00000000-0000-0000-0000-0000000000c3");
    assert_eq!(inv.from.display_name.as_deref(), Some("Guest Example"));
    // No callModalities in the ring: audio.
    assert_eq!(inv.modalities, vec![Modality::Audio]);
    assert!(inv.links.get("attach").is_some());
    assert!(inv.links.get("acceptance").is_none());
    // The ring's mediaAnswer is a placeholder, never a URL to post to.
    assert_eq!(inv.links.get("mediaAnswer"), Some("cc://ma"));
    assert_eq!(
        calling::invitation::own_participant_id(&inv),
        Some("00000000-0000-0000-0000-0000000000f4")
    );
    assert_eq!(calling::invitation::media_leg_id(&inv), None);
}

#[test]
fn attach_asks_for_the_offer_or_carries_ours() {
    let s = session();
    let ask = attach_body(&s, Some("https://conv.example/c"), true);
    assert_eq!(ask["attach"]["requireMediaContent"], true);
    assert!(ask["attach"].get("mediaContent").is_none());

    let ours = attach_offer_body(
        &s,
        Some("https://conv.example/c"),
        &sdp("v=0\r\nour-offer\r\n"),
    );
    let a = &ours["attach"];
    assert_eq!(a["requireMediaContent"], false);
    assert_eq!(a["mediaContent"]["blob"], "v=0\r\nour-offer\r\n");
    assert_eq!(a["mediaContent"]["contentType"], "application/sdp-ngc-1.0");
    assert_eq!(a["mediaContent"]["mediaLegId"], "LEG1");
    assert_eq!(a["mediaContent"]["requiredFeatures"], "nonByPass");
    assert_eq!(a["links"]["mediaAnswer"], s.callbacks.call("mediaAnswer"));
    assert_eq!(a["links"]["end"], s.callbacks.call("end"));
    assert_eq!(ours["additionalActions"][0]["name"], "join");
}

#[test]
fn an_offer_acceptance_names_where_the_answer_goes() {
    let s = session();
    let b = offer_acceptance_body(&s, &[Modality::Audio], &sdp("v=0\r\nour-offer\r\n"));
    let c = &b["callAcceptance"];
    assert_eq!(c["mediaContent"]["blob"], "v=0\r\nour-offer\r\n");
    // Labelled like place_call's offer, which the service took live.
    assert_eq!(c["mediaContent"]["contentType"], "application/sdp-ngc-1.0");
    assert_eq!(c["mediaContent"]["requiredFeatures"], "nonByPass");
    assert_eq!(c["acceptedCallModalities"], json!(["Audio"]));
    assert_eq!(c["links"]["mediaAnswer"], s.callbacks.call("mediaAnswer"));
    assert_eq!(
        c["links"]["mediaAcknowledgement"],
        s.callbacks.call("mediaAcknowledgement")
    );
    assert!(c["links"]["mediaRenegotiation"].is_string());
}

#[test]
fn our_offer_goes_out_as_a_media_negotiation() {
    let s = session();
    let b = negotiation_body(&s, &[Modality::Audio], &sdp("v=0\r\nour-offer\r\n"));
    let n = &b["mediaNegotiation"];
    assert_eq!(n["callModalities"], json!(["Audio"]));
    assert_eq!(n["sender"]["id"], s.me.mri);
    assert_eq!(n["links"]["mediaAnswer"], s.callbacks.call("mediaAnswer"));
    assert_eq!(n["links"]["rejection"], s.callbacks.call("rejection"));
    let m = &n["mediaContent"];
    assert_eq!(m["blob"], "v=0\r\nour-offer\r\n");
    assert_eq!(m["contentType"], "application/sdp-ngc-1.0");
    assert_eq!(m["mediaLegId"], "LEG1");
    assert_eq!(m["requiredFeatures"], "nonByPass");
    assert_eq!(m["newOffer"], true);

    // The acceptance that follows carries no SDP: our offer is already out.
    let a = mediafree_acceptance_body(&s, &[Modality::Audio]);
    let c = &a["callAcceptance"];
    assert_eq!(c["mediaContent"], Value::Null);
    assert_eq!(c["acceptedBy"]["id"], s.me.mri);
    assert_eq!(c["links"]["mediaAnswer"], s.callbacks.call("mediaAnswer"));
    assert!(c["links"]["mediaRenegotiation"].is_string());
}

#[test]
fn an_inline_answer_is_found_where_the_service_may_put_it() {
    for body in [
        json!({"mediaAnswer": {"mediaContent": {"contentType": "application/sdp-ngc-1.0", "blob": "v=0\r\nx\r\n"}}}),
        json!({"callAcceptanceAcknowledgement": {"mediaContent": {"contentType": "application/sdp-ngc-1.0", "blob": "v=0\r\nx\r\n"}}}),
        json!({"mediaContent": {"contentType": "application/sdp-ngc-1.0", "blob": "v=0\r\nx\r\n"}}),
    ] {
        assert_eq!(
            parse_inline_answer(&body),
            Some(sdp("v=0\r\nx\r\n")),
            "{body}"
        );
    }
    assert_eq!(parse_inline_answer(&json!({"links": {}})), None);
    assert_eq!(parse_inline_answer(&Value::Null), None);
}

#[test]
fn modes_parse_and_pick_where_our_offer_goes() {
    assert_eq!(NoOffer::parse("fetch"), Some(NoOffer::FetchOffer));
    assert_eq!(NoOffer::parse("new-offer"), Some(NoOffer::NewOffer));
    assert_eq!(NoOffer::parse("accept-offer"), Some(NoOffer::AcceptOffer));
    assert_eq!(NoOffer::parse("attach-offer"), Some(NoOffer::OfferInAttach));
    assert_eq!(NoOffer::parse("nope"), None);

    // new-offer attaches like the web client, without asking for media.
    let mut a = Answering::new(&no_offer_invitation(), NoOffer::NewOffer);
    assert_eq!(
        a.step(),
        &Step::Attach {
            require_media: false,
            with_offer: false
        }
    );
    a.on_attached(attached_without_offer());
    assert_eq!(a.step(), &Step::SendOffer(new_offer_via()));

    // accept-offer always uses the acceptance.
    let mut a = Answering::new(&no_offer_invitation(), NoOffer::AcceptOffer);
    a.on_attached(attached_without_offer());
    assert_eq!(a.step(), &Step::SendOffer(OfferVia::Acceptance));

    // Without a newOffer link the acceptance is the only place left.
    let mut a = Answering::new(&no_offer_invitation(), NoOffer::FetchOffer);
    let mut bare = attached_without_offer();
    bare.links.0.remove("newOffer");
    a.on_attached(bare);
    assert_eq!(a.step(), &Step::SendOffer(OfferVia::Acceptance));
}

#[test]
fn the_attach_answer_carries_the_callers_offer_and_leg() {
    let a = attach_answer();
    assert!(a.links.get("acceptance").unwrap().ends_with("/accept?i=1"));
    assert!(a.links.get("newOffer").is_some());
    let offer = a.offer.unwrap();
    assert_eq!(offer.content_type, "application/sdp-ngc-1.0");
    assert!(offer.blob.contains("m=audio"));
    assert_eq!(
        a.media_leg_id.as_deref(),
        Some("FEDCBA9876543210FEDCBA9876543210")
    );
    let bare =
        parse_attach(&json!({"callInvitation": {"links": {"acceptance": "https://a"}}})).unwrap();
    assert_eq!((bare.offer, bare.media_leg_id), (None, None));
    assert!(parse_attach(&json!({})).is_err());
}

#[test]
fn the_attach_answer_names_the_conversation_controller() {
    use calling::signaling::bodies::CONVERSATION_CONTROLLER;
    let a = parse_attach(&json!({
        "callInvitation": {"links": {"acceptance": "https://a"}},
        "additionalActionResponses": [
            {"output": {"conversationController": "https://cc.example/conv/1"}}
        ],
    }))
    .unwrap();
    assert_eq!(
        a.links.get(CONVERSATION_CONTROLLER),
        Some("https://cc.example/conv/1")
    );
    let plain = parse_attach(&json!({
        "callInvitation": {"links": {}},
        "additionalActionResponses": [{"output": {"conversationController": "http://x.invalid"}}],
    }))
    .unwrap();
    assert_eq!(plain.links.get(CONVERSATION_CONTROLLER), None);
}

#[test]
fn an_answer_may_come_inline_with_the_acknowledgement() {
    let body = json!({"callAcceptanceAcknowledgement": {
        "links": {"callLeg": "https://cc.example/leg"},
        "mediaContent": {"contentType": "application/sdp-ngc-1.0", "blob": "v=0\r\nx\r\n"},
    }});
    assert_eq!(parse_acceptance_answer(&body), Some(sdp("v=0\r\nx\r\n")));
    assert_eq!(
        parse_acceptance_answer(&json!({"callAcceptanceAcknowledgement": {"links": {}}})),
        None
    );
}

#[test]
fn the_media_answer_callback_on_our_session_parses() {
    let s = session();
    let (url, body) = (
        s.callbacks.call("mediaAnswer"),
        json!({"mediaAnswer": {"mediaContent": {"blob": "v=0\r\na\r\n"}, "links": {}}}),
    );
    match classify(&url, &body) {
        NgcPush::Call { session_id, event } => {
            assert_eq!(session_id, "sess-1");
            let CallEvent::MediaAnswer { answer, .. } = event else {
                panic!("not a media answer: {event:?}")
            };
            assert_eq!(answer.blob, "v=0\r\na\r\n");
            assert_eq!(answer.content_type, "application/sdp");
        }
        other => panic!("not a callback: {other:?}"),
    }
}

// --- the state machine ---------------------------------------------------------------------

#[test]
fn a_ring_with_an_offer_and_acceptance_is_answered_at_once() {
    let mut a = Answering::new(&with_offer(no_offer_invitation(), true), NoOffer::default());
    assert_eq!(a.step(), &Step::AnswerOffer(sdp("v=0\r\nring-offer\r\n")));
    a.on_accepted(links(&[("callLeg", "https://cc.example/leg")]), None);
    assert_eq!(a.step(), &Step::Adopt);
    assert_eq!(a.remote_answer(), None);
    assert!(a.links().get("callLeg").is_some());
}

#[test]
fn a_forked_ring_with_an_offer_attaches_without_asking() {
    let mut a = Answering::new(
        &with_offer(no_offer_invitation(), false),
        NoOffer::default(),
    );
    assert_eq!(
        a.step(),
        &Step::Attach {
            require_media: false,
            with_offer: false
        }
    );
    // The ring's offer wins over anything the attach answer carries.
    a.on_attached(attach_answer());
    assert_eq!(a.step(), &Step::AnswerOffer(sdp("v=0\r\nring-offer\r\n")));
}

#[test]
fn without_an_offer_attach_fetches_it_then_we_answer() {
    let mut a = Answering::new(&no_offer_invitation(), NoOffer::FetchOffer);
    assert_eq!(
        a.step(),
        &Step::Attach {
            require_media: true,
            with_offer: false
        }
    );
    let attached = attach_answer();
    let offer = attached.offer.clone().unwrap();
    a.on_attached(attached);
    assert_eq!(a.step(), &Step::AnswerOffer(offer));
    assert_eq!(a.media_leg_id(), Some("FEDCBA9876543210FEDCBA9876543210"));
    // The real mediaAnswer link replaced the ring's placeholder.
    assert!(
        a.links()
            .get("mediaAnswer")
            .unwrap()
            .starts_with("https://")
    );
    a.on_accepted(links(&[("callLeg", "https://cc.example/leg")]), None);
    assert_eq!(a.step(), &Step::Adopt);
    assert!(!a.offering());
}

#[test]
fn when_no_offer_comes_we_offer_and_wait_for_the_media_answer() {
    let mut a = Answering::new(&no_offer_invitation(), NoOffer::FetchOffer);
    a.on_attached(attached_without_offer());
    assert_eq!(a.step(), &Step::SendOffer(new_offer_via()));
    a.on_accepted(links(&[("callLeg", "https://cc.example/leg")]), None);
    assert_eq!(a.step(), &Step::AwaitAnswer);
    // Unrelated callbacks change nothing.
    a.on_event(&CallEvent::Progress {
        status: "ringing".into(),
    });
    assert_eq!(a.step(), &Step::AwaitAnswer);
    a.on_event(&media_answer_event());
    assert_eq!(a.step(), &Step::Adopt);
    assert!(a.offering());
    assert_eq!(a.remote_answer(), Some(&sdp("v=0\r\ncaller-answer\r\n")));
    let l = a.into_links();
    assert!(l.get("callLeg").is_some() && l.get("mediaAcknowledgement").is_some());
}

#[test]
fn an_inline_or_early_answer_skips_the_wait() {
    let mut inline = Answering::new(&no_offer_invitation(), NoOffer::FetchOffer);
    inline.on_attached(attached_without_offer());
    inline.on_accepted(Links::default(), Some(sdp("v=0\r\ninline\r\n")));
    assert_eq!(inline.step(), &Step::Adopt);
    assert_eq!(inline.remote_answer(), Some(&sdp("v=0\r\ninline\r\n")));

    // The callback raced the acceptance's HTTP answer.
    let mut early = Answering::new(&no_offer_invitation(), NoOffer::FetchOffer);
    early.on_attached(attached_without_offer());
    early.on_event(&media_answer_event());
    assert_eq!(early.step(), &Step::SendOffer(new_offer_via()));
    early.on_accepted(Links::default(), None);
    assert_eq!(early.step(), &Step::Adopt);
}

#[test]
fn offer_in_attach_stays_the_offerer() {
    let mut a = Answering::new(&no_offer_invitation(), NoOffer::OfferInAttach);
    assert_eq!(
        a.step(),
        &Step::Attach {
            require_media: false,
            with_offer: true
        }
    );
    // Even if an offer came back, our offer is already out on this peer.
    a.on_attached(attach_answer());
    assert_eq!(a.step(), &Step::SendOffer(OfferVia::Attach));
    a.on_accepted(Links::default(), None);
    a.on_event(&media_answer_event());
    assert_eq!(a.step(), &Step::Adopt);
}

#[test]
fn no_answer_an_end_or_no_acceptance_link_fails() {
    let waiting = || {
        let mut a = Answering::new(&no_offer_invitation(), NoOffer::FetchOffer);
        a.on_attached(attached_without_offer());
        a.on_accepted(Links::default(), None);
        a
    };
    let mut timed_out = waiting();
    timed_out.on_timeout();
    assert!(matches!(timed_out.step(), Step::Failed(_)));

    let mut ended = waiting();
    ended.on_event(&CallEvent::Ended(EndReason::default()));
    assert!(matches!(ended.step(), Step::Failed(_)));

    let mut no_link = Answering::new(&no_offer_invitation(), NoOffer::FetchOffer);
    no_link.on_attached(Attached::default());
    assert!(matches!(no_link.step(), Step::Failed(_)));

    // A timeout outside the wait means nothing.
    let mut answering =
        Answering::new(&with_offer(no_offer_invitation(), true), NoOffer::default());
    answering.on_timeout();
    assert!(matches!(answering.step(), Step::AnswerOffer(_)));
}
