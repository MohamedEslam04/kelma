//! A `MediaSession` with a screen-sharing line negotiating against Teams-shaped SDP, offline
//! (host candidates only, nothing is ever dialled).

use calling::media::{MediaOptions, MediaSession};
use calling::video::VideoRole;

const FULL_REOFFER: &str = include_str!("fixtures/media/teams_meeting_full_reoffer.sdp");

fn sharing() -> MediaOptions {
    MediaOptions {
        udp_addrs: vec!["127.0.0.1:0".into()],
        video: vec![VideoRole::ApplicationSharing],
        ..Default::default()
    }
}

fn sections(sdp: &str) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    for line in sdp.split("\r\n").filter(|l| !l.is_empty()) {
        if line.starts_with("m=") {
            out.push(vec![line]);
        } else if let Some(s) = out.last_mut() {
            s.push(line);
        }
    }
    out
}

#[tokio::test]
async fn a_sharing_session_offers_audio_and_a_labelled_h264_line() {
    let (session, _events) = MediaSession::with_options(vec![], sharing()).await.unwrap();
    assert!(
        session
            .video_sender(VideoRole::ApplicationSharing)
            .is_some()
    );
    assert!(session.video_sender(VideoRole::MainVideo).is_none());
    let offer = session.create_offer().await.unwrap();
    let s = sections(&offer);
    assert_eq!(s.len(), 2, "{offer}");
    assert!(s[1][0].starts_with("m=video ") && s[1][0].contains(" RTP/SAVP "));
    assert!(
        s[1].contains(&"a=label:applicationsharing-video"),
        "{offer}"
    );
    assert!(
        s[1].iter()
            .any(|l| l.starts_with("a=rtpmap:") && l.ends_with(" H264/90000"))
    );
    assert!(s[1].iter().any(|l| l.starts_with("a=x-ssrc-range:")));
    assert!(s[1].contains(&"a=sendrecv"));
    assert_eq!(
        session
            .video_mid(VideoRole::ApplicationSharing)
            .await
            .as_deref(),
        Some("1")
    );
    session.close().await.unwrap();
}

#[tokio::test]
async fn a_sharing_session_answers_the_meeting_re_offer_with_its_sharing_line() {
    let (session, _events) = MediaSession::with_options(vec![], sharing()).await.unwrap();
    let answer = session.accept_offer(FULL_REOFFER).await.unwrap();
    let s = sections(&answer);
    assert_eq!(s.len(), 13, "{answer}");
    let share = s
        .iter()
        .find(|x| x.contains(&"a=mid:3"))
        .expect("sharing line");
    assert!(!share[0].starts_with("m=video 0 "), "{answer}");
    assert!(
        share[0].ends_with(" 107"),
        "Teams' H.264 payload type: {share:?}"
    );
    assert!(share.contains(&"a=label:applicationsharing-video"));
    assert!(share.contains(&"a=recvonly"), "{share:?}");
    assert!(share.iter().any(|l| l.starts_with("a=x-ssrc-range:")));
    let gallery = s.iter().find(|x| x.contains(&"a=mid:2")).unwrap();
    assert_eq!(gallery[0], "m=video 0 RTP/SAVP 34");
    let bundle = answer
        .split("\r\n")
        .find(|l| l.starts_with("a=group:BUNDLE"))
        .unwrap();
    assert_eq!(bundle, "a=group:BUNDLE 1 3");
    assert_eq!(
        session
            .video_mid(VideoRole::ApplicationSharing)
            .await
            .as_deref(),
        Some("3")
    );
    session.close().await.unwrap();
}

#[tokio::test]
async fn video_written_before_negotiation_goes_nowhere_without_an_error() {
    let (session, _events) = MediaSession::with_options(vec![], sharing()).await.unwrap();
    let sender = session.video_sender(VideoRole::ApplicationSharing).unwrap();
    let r = sender
        .send(
            vec![0, 0, 0, 1, 0x65, 1],
            std::time::Duration::from_millis(66),
        )
        .await;
    assert!(r.is_ok(), "{r:?}");
    session.close().await.unwrap();
}
