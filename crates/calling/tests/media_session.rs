//! A `MediaSession` negotiating against Teams-shaped SDP, offline (host candidates only, nothing
//! is ever dialled).

use calling::media::{MediaOptions, MediaSession};

const P2P_OFFER: &str = include_str!("fixtures/media/teams_p2p_offer.sdp");
const WEB_OFFER: &str = include_str!("fixtures/media/teams_web_offer.sdp");
const MEETING_ANSWER: &str = include_str!("fixtures/media/teams_meeting_answer.sdp");

fn local() -> MediaOptions {
    MediaOptions {
        udp_addrs: vec!["127.0.0.1:0".into()],
    }
}

fn section_lines(sdp: &str) -> Vec<&str> {
    sdp.split("\r\n").filter(|l| !l.is_empty()).collect()
}

#[tokio::test]
async fn offer_is_complete_and_in_the_teams_dialect() {
    let (session, _events) = MediaSession::with_options(vec![], local()).await.unwrap();
    let offer = session.create_offer().await.unwrap();
    let l = section_lines(&offer);
    let m = l
        .iter()
        .find(|x| x.starts_with("m=audio "))
        .expect("audio line");
    assert!(m.contains(" RTP/SAVP "), "{m}");
    assert!(m.split(' ').nth(3).is_some_and(|pt| pt == "111"), "{m}");
    assert!(l.contains(&"a=rtpmap:111 opus/48000/2"));
    assert!(l.contains(&"a=fmtp:111 minptime=10;useinbandfec=1"));
    assert!(l.contains(&"a=setup:actpass"));
    assert!(l.iter().any(|x| x.starts_with("a=fingerprint:sha-256 ")));
    assert!(l.contains(&"a=label:main-audio"));
    assert!(l.iter().any(|x| x.starts_with("a=x-ssrc-range:")));
    // Non-trickle: the candidates are inside the one blob.
    assert!(
        l.iter()
            .any(|x| x.starts_with("a=candidate:") && x.contains("127.0.0.1")),
        "{offer}"
    );
    session.close().await.unwrap();
}

#[tokio::test]
async fn answers_a_native_teams_p2p_offer() {
    let (session, _events) = MediaSession::with_options(vec![], local()).await.unwrap();
    let answer = session.accept_offer(P2P_OFFER).await.unwrap();
    let l = section_lines(&answer);
    let audio = l
        .iter()
        .find(|x| x.starts_with("m=audio "))
        .unwrap_or_else(|| panic!("no audio line in {answer}"));
    assert!(audio.contains(" RTP/SAVP "), "{audio}");
    // Opus is 102 in a desktop client's offer (111 is SIREN there): the answer follows it.
    assert_eq!(audio.split(' ').nth(3), Some("102"), "{answer}");
    assert!(l.contains(&"a=rtpmap:102 opus/48000/2"), "{answer}");
    assert!(l.contains(&"a=mid:audio_0"));
    assert!(l.contains(&"a=setup:active"));
    assert!(l.iter().any(|x| x.starts_with("a=candidate:")));
    // The video line is declined: port 0.
    assert!(l.iter().any(|x| x.starts_with("m=video 0 ")), "{answer}");
    session.close().await.unwrap();
}

#[tokio::test]
async fn answers_a_teams_web_offer() {
    let (session, _events) = MediaSession::with_options(vec![], local()).await.unwrap();
    let answer = session.accept_offer(WEB_OFFER).await.unwrap();
    let l = section_lines(&answer);
    let audio = l.iter().find(|x| x.starts_with("m=audio ")).unwrap();
    assert_eq!(audio.split(' ').nth(3), Some("111"), "{answer}");
    assert!(l.contains(&"a=label:main-audio"));
    session.close().await.unwrap();
}

#[tokio::test]
async fn takes_a_teams_media_server_answer() {
    let (session, _events) = MediaSession::with_options(vec![], local()).await.unwrap();
    session.create_offer().await.unwrap();
    session.set_answer(MEETING_ANSWER).await.unwrap();
    session.close().await.unwrap();
}

#[tokio::test]
async fn rejects_garbage() {
    let (session, _events) = MediaSession::with_options(vec![], local()).await.unwrap();
    assert!(session.accept_offer("hello").await.is_err());
    session.close().await.unwrap();
}
