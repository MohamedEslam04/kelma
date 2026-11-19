//! The SDP rewrites for the data channel (live captions): Teams' `m=x-data` line read as a
//! WebRTC SCTP section, and our SCTP section written back as Teams' `x-data` line.

use calling::media::sdp::{
    from_teams_with, from_teams_with_data, to_teams_answer_with, to_teams_answer_with_data,
    to_teams_offer,
};

/// The media servers' re-offer after a lobby admission (shape from a live kelma run).
const FULL_REOFFER: &str = include_str!("fixtures/media/teams_meeting_full_reoffer.sdp");

/// A media server's offer with audio and the data line, as the fixtures write them.
const TEAMS_OFFER: &str = "v=0\r\no=- 1 0 IN IP4 198.51.100.20\r\ns=session\r\nt=0 0\r\n\
a=group:BUNDLE 1 4\r\n\
m=audio 3478 RTP/AVP 111\r\nc=IN IP4 198.51.100.20\r\na=rtcp:3478\r\na=setup:actpass\r\n\
a=ice-ufrag:aUdI\r\na=ice-pwd:audioaudioaudioaudioaudi\r\na=rtcp-mux\r\n\
a=candidate:1 1 UDP 54001663 198.51.100.20 3478 typ relay raddr 10.0.0.1 rport 3478\r\n\
a=label:main-audio\r\na=mid:1\r\na=rtpmap:111 opus/48000/2\r\n\
a=fingerprint:sha-256 11:22\r\n\
m=x-data 3478 RTP/AVP 127 126\r\nc=IN IP4 198.51.100.20\r\na=rtcp:3478\r\na=setup:actpass\r\n\
a=ice-ufrag:aUdI\r\na=ice-pwd:audioaudioaudioaudioaudi\r\na=rtcp-mux\r\n\
a=label:data\r\na=mid:4\r\na=rtpmap:127 x-data/90000\r\na=rtpmap:126 rtx/90000\r\n\
a=fmtp:126 apt=127\r\na=x-data-protocol:sctp\r\na=fingerprint:sha-256 11:22\r\n";

/// What webrtc-rs answers to `TEAMS_OFFER` once it was read with the data line.
const OUR_ANSWER: &str = "v=0\r\no=- 9 1 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
a=group:BUNDLE 1 4\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=setup:active\r\na=mid:1\r\n\
a=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\na=fingerprint:sha-256 AA:BB\r\n\
a=rtpmap:111 opus/48000/2\r\na=ssrc:42 cname:x\r\na=sendrecv\r\na=rtcp-mux\r\n\
a=candidate:1 1 udp 2130706431 192.0.2.10 50000 typ host\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 0.0.0.0\r\na=setup:active\r\n\
a=mid:4\r\na=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\na=fingerprint:sha-256 AA:BB\r\n\
a=sctp-port:5000\r\na=max-message-size:262144\r\n";

/// A webrtc-rs offer with audio and a data channel.
const OUR_OFFER: &str = "v=0\r\no=- 9 1 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n\
a=group:BUNDLE 0 1\r\na=fingerprint:sha-256 AA:BB\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=setup:actpass\r\na=mid:0\r\n\
a=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\n\
a=rtpmap:111 opus/48000/2\r\na=ssrc:42 cname:x\r\na=sendrecv\r\na=rtcp-mux\r\n\
a=candidate:1 1 udp 2130706431 192.0.2.10 50000 typ host\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 0.0.0.0\r\na=setup:actpass\r\n\
a=mid:1\r\na=ice-ufrag:ours\r\na=ice-pwd:ourpassword\r\n\
a=sctp-port:5000\r\na=max-message-size:262144\r\n";

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

fn has(section: &[&str], line: &str) -> bool {
    section.contains(&line)
}

#[test]
fn the_data_line_becomes_an_sctp_section_when_the_session_has_the_channel() {
    let browser = from_teams_with_data(TEAMS_OFFER, &[], true, true).unwrap();
    let s = sections(&browser);
    assert_eq!(s.len(), 2, "{browser}");
    let data = &s[1];
    assert_eq!(
        data[0],
        "m=application 3478 UDP/DTLS/SCTP webrtc-datachannel"
    );
    for line in [
        "a=mid:4",
        "a=setup:actpass",
        "a=ice-ufrag:aUdI",
        "a=sctp-port:5000",
        "a=max-message-size:262144",
    ] {
        assert!(has(data, line), "{line} missing: {browser}");
    }
    // Nothing RTP-only, and no direction (an SCTP section has none).
    assert!(
        !data.iter().any(|l| l.starts_with("a=rtpmap")
            || l.starts_with("a=x-")
            || l.starts_with("a=label")
            || l.starts_with("a=rtcp")
            || *l == "a=sendrecv"),
        "{browser}"
    );
    assert!(browser.contains("a=group:BUNDLE 1 4\r\n"), "{browser}");
}

#[test]
fn without_the_channel_the_data_line_still_goes() {
    for browser in [
        from_teams_with(TEAMS_OFFER, &[], true).unwrap(),
        from_teams_with_data(TEAMS_OFFER, &[], true, false).unwrap(),
    ] {
        assert!(!browser.contains("m=application"), "{browser}");
        assert!(!browser.contains("m=x-data"), "{browser}");
        assert!(browser.contains("a=group:BUNDLE 1\r\n"), "{browser}");
    }
}

#[test]
fn the_full_reoffers_data_line_is_bundled_onto_the_audio_transport() {
    let browser = from_teams_with_data(FULL_REOFFER, &[], true, true).unwrap();
    let s = sections(&browser);
    let data = s
        .iter()
        .find(|s| s[0].starts_with("m=application"))
        .unwrap_or_else(|| panic!("no SCTP section: {browser}"));
    assert_eq!(
        data[0],
        "m=application 3480 UDP/DTLS/SCTP webrtc-datachannel"
    );
    assert!(has(data, "a=ice-ufrag:aUdI"), "{browser}");
    assert!(!data.iter().any(|l| l.contains("dAtA")), "{browser}");
    assert!(
        !data.iter().any(|l| l.starts_with("a=candidate")),
        "{browser}"
    );
    assert!(has(data, "a=mid:4"), "{browser}");
}

#[test]
fn a_declined_data_line_in_an_answer_is_still_answered() {
    let answer = TEAMS_OFFER.replace("m=x-data 3478 ", "m=x-data 0 ");
    let browser = from_teams_with_data(&answer, &[], false, true).unwrap();
    let s = sections(&browser);
    assert_eq!(s.len(), 2, "{browser}");
    assert_eq!(s[1][0], "m=application 0 UDP/DTLS/SCTP webrtc-datachannel");
}

#[test]
fn our_answer_gives_the_data_line_in_teams_form() {
    let teams = to_teams_answer_with_data(OUR_ANSWER, TEAMS_OFFER, &[], true);
    let s = sections(&teams);
    assert_eq!(s.len(), 2, "{teams}");
    let data = &s[1];
    assert_eq!(data[0], "m=x-data 50000 RTP/SAVP 127 126");
    for line in [
        "c=IN IP4 192.0.2.10",
        "a=x-data-protocol:sctp",
        "a=rtpmap:127 x-data/90000",
        "a=rtpmap:126 rtx/90000",
        "a=fmtp:126 apt=127",
        "a=rtcp-mux",
        "a=sendrecv",
        "a=mid:4",
        "a=label:data",
        "a=sctp-port:5000",
    ] {
        assert!(has(data, line), "{line} missing: {teams}");
    }
    assert!(
        data.iter().any(|l| l.starts_with("a=x-ssrc-range:")),
        "{teams}"
    );
    // An answer carries no `a=rtcp:<port>`.
    assert!(!data.iter().any(|l| l.starts_with("a=rtcp:")), "{teams}");
}

#[test]
fn without_the_channel_our_answer_stubs_the_data_line() {
    let teams = to_teams_answer_with(OUR_ANSWER, TEAMS_OFFER, &[]);
    let s = sections(&teams);
    assert_eq!(s[1][0], "m=x-data 0 RTP/SAVP 34");
}

#[test]
fn our_offer_writes_the_data_channel_as_teams_x_data_line() {
    let teams = to_teams_offer(OUR_OFFER);
    let s = sections(&teams);
    assert_eq!(s.len(), 2, "{teams}");
    let data = &s[1];
    assert_eq!(data[0], "m=x-data 50000 RTP/SAVP 127 126");
    for line in [
        "a=x-data-protocol:sctp",
        "a=rtpmap:127 x-data/90000",
        "a=mid:1",
        "a=label:data",
        "a=fingerprint:sha-256 AA:BB",
    ] {
        assert!(has(data, line), "{line} missing: {teams}");
    }
    // The data line names no RTCP port, as rusty-teams writes it.
    assert!(!data.iter().any(|l| l.starts_with("a=rtcp:")), "{teams}");
    let range = data
        .iter()
        .find_map(|l| l.strip_prefix("a=x-ssrc-range:"))
        .unwrap();
    let (lo, hi) = range.split_once('-').unwrap();
    assert_eq!(lo, hi);
}
