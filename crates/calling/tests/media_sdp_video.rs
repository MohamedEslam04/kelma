//! The SDP rewrites for a session with video lines (screen share; the camera later): which
//! sections webrtc-rs is given, how a media server's re-offer is bundled onto the audio transport,
//! and the directions and labels Teams sees.

use calling::media::sdp::{
    answer_direction, from_teams, from_teams_with, mid_of_role, to_teams_answer_with,
    to_teams_offer, with_role_direction,
};
use calling::video::VideoRole;

/// The media servers' re-offer after a lobby admission (shape from a live kelma run).
const FULL_REOFFER: &str = include_str!("fixtures/media/teams_meeting_full_reoffer.sdp");
const SHARE: &[VideoRole] = &[VideoRole::ApplicationSharing];

fn sections(sdp: &str) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    for line in sdp.lines().filter(|l| !l.is_empty()) {
        if line.starts_with("m=") {
            out.push(vec![line]);
        } else if let Some(s) = out.last_mut() {
            s.push(line);
        }
    }
    out
}

fn value<'a>(section: &[&'a str], name: &str) -> Option<&'a str> {
    section
        .iter()
        .find_map(|l| l.strip_prefix(&format!("a={name}:")))
}

fn has(section: &[&str], line: &str) -> bool {
    section.contains(&line)
}

#[test]
fn audio_only_sessions_still_drop_every_video_line() {
    let browser = from_teams(FULL_REOFFER).unwrap();
    assert!(
        sections(&browser)
            .iter()
            .all(|s| s[0].starts_with("m=audio"))
    );
}

#[test]
fn a_share_session_keeps_audio_and_the_first_sharing_line_only() {
    let browser = from_teams_with(FULL_REOFFER, SHARE, true).unwrap();
    let kept = sections(&browser);
    assert_eq!(kept.len(), 2, "{browser}");
    assert!(kept[0][0].starts_with("m=audio"));
    assert!(kept[1][0].starts_with("m=video"));
    assert_eq!(value(&kept[1], "label"), Some("applicationsharing-video"));
    assert_eq!(value(&kept[1], "mid"), Some("3"));
    assert!(browser.contains("a=group:BUNDLE 1 3\r\n"), "{browser}");
}

#[test]
fn the_sharing_line_is_bundled_onto_the_audio_transport() {
    let browser = from_teams_with(FULL_REOFFER, SHARE, true).unwrap();
    let kept = sections(&browser);
    let audio_ufrag = value(&kept[0], "ice-ufrag").unwrap();
    assert_eq!(value(&kept[1], "ice-ufrag"), Some(audio_ufrag));
    assert_eq!(value(&kept[1], "ice-pwd"), value(&kept[0], "ice-pwd"));
    assert!(
        !kept[1].iter().any(|l| l.starts_with("a=candidate:")),
        "the sharing transport's own candidates go"
    );
    assert!(!kept[1].iter().any(|l| l.starts_with("a=rtcp:")));
    assert!(kept[0].iter().any(|l| l.starts_with("a=candidate:")));
}

#[test]
fn a_sendonly_sharing_offer_is_read_as_sendrecv_so_we_can_send_later() {
    let browser = from_teams_with(FULL_REOFFER, SHARE, true).unwrap();
    let share = &sections(&browser)[1];
    assert!(has(share, "a=sendrecv"), "{share:?}");
    assert!(!has(share, "a=sendonly"));
    // An answer is taken as it is.
    let answer = from_teams_with(FULL_REOFFER, SHARE, false).unwrap();
    assert!(has(&sections(&answer)[1], "a=sendonly"));
}

#[test]
fn the_camera_role_takes_the_first_main_video_line() {
    let browser = from_teams_with(FULL_REOFFER, &[VideoRole::MainVideo], true).unwrap();
    let kept = sections(&browser);
    assert_eq!(kept.len(), 2);
    assert_eq!(value(&kept[1], "mid"), Some("2"));
    assert_eq!(value(&kept[1], "label"), Some("main-video"));
}

#[test]
fn answer_directions_follow_what_the_offer_allows() {
    assert_eq!(answer_direction("sendonly", "sendrecv"), "recvonly");
    assert_eq!(answer_direction("recvonly", "sendrecv"), "sendonly");
    assert_eq!(answer_direction("sendrecv", "sendrecv"), "sendrecv");
    assert_eq!(answer_direction("inactive", "sendrecv"), "inactive");
    assert_eq!(answer_direction("sendrecv", "recvonly"), "recvonly");
}

/// A webrtc-rs answer to the share session's reading of `FULL_REOFFER` (shape only).
const OUR_ANSWER: &str = "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
a=fingerprint:sha-256 AA:BB\r\na=group:BUNDLE 1 3\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=setup:active\r\na=mid:1\r\n\
a=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\na=rtcp-mux\r\na=rtpmap:111 opus/48000/2\r\n\
a=ssrc:1111 cname:kelma\r\na=sendrecv\r\n\
a=candidate:1 1 udp 2130706431 192.0.2.10 50000 typ host\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 107\r\nc=IN IP4 0.0.0.0\r\na=setup:active\r\na=mid:3\r\n\
a=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\na=rtcp-mux\r\na=rtpmap:107 H264/90000\r\n\
a=fmtp:107 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=msid:applicationsharing-video kelma-share\r\na=ssrc:2222 cname:kelma\r\na=sendrecv\r\n";

#[test]
fn the_answer_has_every_offered_line_and_a_recvonly_sharing_line() {
    let teams = to_teams_answer_with(OUR_ANSWER, FULL_REOFFER, SHARE);
    let ours = sections(&teams);
    assert_eq!(ours.len(), sections(FULL_REOFFER).len(), "{teams}");
    let share = ours
        .iter()
        .find(|s| value(s, "mid") == Some("3"))
        .expect("sharing line answered");
    assert!(!share[0].starts_with("m=video 0 "), "{share:?}");
    assert!(has(share, "a=recvonly"), "{share:?}");
    assert_eq!(value(share, "label"), Some("applicationsharing-video"));
    assert_eq!(value(share, "x-ssrc-range"), Some("2222-2222"));
    // The gallery lines are declined.
    let gallery = ours.iter().find(|s| value(s, "mid") == Some("2")).unwrap();
    assert!(gallery[0].starts_with("m=video 0 "));
}

/// A webrtc-rs offer of a share session (shape only).
const OUR_OFFER: &str = "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
a=fingerprint:sha-256 AA:BB\r\na=group:BUNDLE 0 1\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=setup:actpass\r\na=mid:0\r\n\
a=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\na=rtcp-mux\r\na=rtpmap:111 opus/48000/2\r\n\
a=msid:kelma-1111 kelma-audio\r\na=ssrc:1111 cname:kelma\r\na=sendrecv\r\n\
a=candidate:1 1 udp 2130706431 192.0.2.10 50000 typ host\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 107\r\nc=IN IP4 0.0.0.0\r\na=setup:actpass\r\na=mid:1\r\n\
a=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\na=rtcp-mux\r\na=rtpmap:107 H264/90000\r\n\
a=msid:applicationsharing-video kelma-share\r\na=ssrc:2222 cname:kelma\r\na=sendrecv\r\n";

#[test]
fn our_offer_labels_the_sharing_line_from_its_stream() {
    let teams = to_teams_offer(OUR_OFFER);
    let ours = sections(&teams);
    assert_eq!(value(&ours[0], "label"), Some("main-audio"));
    assert_eq!(value(&ours[1], "label"), Some("applicationsharing-video"));
    assert_eq!(value(&ours[1], "x-ssrc-range"), Some("2222-2222"));
    assert!(
        ours[1][0].starts_with("m=video 50000 RTP/SAVP 107"),
        "{:?}",
        ours[1]
    );
    assert_eq!(
        mid_of_role(&teams, VideoRole::ApplicationSharing).as_deref(),
        Some("1")
    );
    assert_eq!(mid_of_role(&teams, VideoRole::MainVideo), None);
}

#[test]
fn a_re_offer_states_the_sharing_direction() {
    let teams = to_teams_offer(OUR_OFFER);
    let sharing = with_role_direction(&teams, VideoRole::ApplicationSharing, "sendonly");
    let ours = sections(&sharing);
    assert!(has(&ours[1], "a=sendonly") && !has(&ours[1], "a=sendrecv"));
    assert!(has(&ours[0], "a=sendrecv"), "audio untouched");
    let stopped = with_role_direction(&sharing, VideoRole::ApplicationSharing, "inactive");
    assert!(has(&sections(&stopped)[1], "a=inactive"));
    assert_eq!(stopped.lines().count(), sharing.lines().count());
}
