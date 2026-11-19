//! Switching video lines with the service: `UpdateMediaDescriptions` (meetings), the 1:1
//! re-offer, and the media controller's sender control (offline, pure bodies).

use calling::signaling::video::{
    MediaDescription, negotiation_tag, parse_sender_controls, update_media_descriptions_body,
    video_modalities, video_renegotiation_body,
};
use calling::signaling::{CallSession, CallbackLinks, LocalEndpoint};
use calling::video::VideoRole;
use calling::{Modality, SdpBlob};
use serde_json::json;

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

#[test]
fn starting_a_share_flips_the_sharing_line_to_sendonly_with_its_label() {
    let lines = [
        MediaDescription::line("2", VideoRole::MainVideo, false),
        MediaDescription::line("3", VideoRole::ApplicationSharing, true),
    ];
    let tag = negotiation_tag(&session(), VideoRole::ApplicationSharing, true);
    assert_eq!(tag, "part-1;ss_1");
    let body = update_media_descriptions_body(&lines, &tag, 4);
    assert_eq!(
        body,
        json!({"UpdateMediaDescriptions": {"mediaDescriptions": {
            "descriptions": [
                {"mid": "2", "direction": "recvonly"},
                {"mid": "3", "direction": "sendonly", "label": "applicationsharing-video"},
            ],
            "negotiationTag": "part-1;ss_1",
            "requestId": 4,
        }}})
    );
}

#[test]
fn stopping_a_share_leaves_the_line_receive_only_without_a_label() {
    let line = MediaDescription::line("11", VideoRole::ApplicationSharing, false);
    assert_eq!(line.wire(), json!({"mid": "11", "direction": "recvonly"}));
}

#[test]
fn the_camera_line_sends_sendrecv_and_tags_its_state() {
    let on = MediaDescription::line("1", VideoRole::MainVideo, true);
    assert_eq!(
        on.wire(),
        json!({"mid": "1", "direction": "sendrecv", "label": "main-video"})
    );
    assert_eq!(
        negotiation_tag(&session(), VideoRole::MainVideo, true),
        "part-1;v_2"
    );
    assert_eq!(
        negotiation_tag(&session(), VideoRole::MainVideo, false),
        "part-1;v_1"
    );
}

#[test]
fn a_share_re_offer_names_the_screen_sharer_modality() {
    assert_eq!(
        video_modalities(&[VideoRole::ApplicationSharing]),
        vec![Modality::Audio, Modality::ScreenSharing]
    );
    assert_eq!(video_modalities(&[]), vec![Modality::Audio]);
    let s = session();
    let offer = SdpBlob::ngc("v=0\r\n");
    let body = video_renegotiation_body(
        &s,
        &video_modalities(&[VideoRole::ApplicationSharing]),
        &offer,
        "part-1;ss_1",
    );
    let n = &body["mediaNegotiation"];
    assert_eq!(n["callModalities"], json!(["Audio", "ScreenSharer"]));
    assert_eq!(n["sender"]["id"], json!(s.me.mri));
    assert_eq!(
        n["links"]["mediaAnswer"],
        json!(s.callbacks.call("mediaAnswer"))
    );
    assert_eq!(
        n["links"]["rejection"],
        json!(s.callbacks.call("rejection"))
    );
    let media = &n["mediaContent"];
    assert_eq!(media["blob"], json!("v=0\r\n"));
    assert_eq!(media["mediaLegId"], json!("LEG1"));
    assert_eq!(media["requiredFeatures"], json!("nonByPass"));
    assert_eq!(media["negotiationTag"], json!("part-1;ss_1"));
    assert_eq!(media["newOffer"], json!(false));
}

#[test]
fn sender_control_names_the_source_its_limits_and_keyframe_requests() {
    let body = json!({"controlVideoStreaming": {"controlInfo": [
        {"sourceId": 212, "fmtParams": "max-fs=8160;max-fps=1500;max-br=2500;KeyFrame=1"},
        {"sourceId": 202, "fmtParams": "max-fs=920"},
    ]}});
    let controls = parse_sender_controls(&body);
    assert_eq!(controls.len(), 2);
    assert_eq!(controls[0].source_id, 212);
    assert!(controls[0].keyframe());
    assert_eq!(controls[0].param("max-fs"), Some("8160"));
    assert_eq!(controls[0].param("max-fps"), Some("1500"));
    assert!(!controls[1].keyframe());
    assert!(parse_sender_controls(&json!({})).is_empty());
}
