//! The meeting's data channel between two `MediaSession`s on localhost, with the roles a Teams
//! meeting gives them: we offer (`a=setup:actpass`), the media server answers `a=setup:passive`,
//! so we are the DTLS client and the server is the DTLS server.
//!
//! As DTLS client, webrtc-rs 0.21 starts the SCTP association itself (it sends the INIT) and its
//! SCTP endpoint has no server config, so an INIT from the other side is refused (`refusing first
//! packet due to empty server_config`). The association still comes up when the other side
//! answers our INIT, as an RFC 4960 §5.2.1 peer does even while its own INIT is out.

use std::time::Duration;

use calling::media::{MediaEvents, MediaOptions, MediaSession, MediaState};
use tokio::time::timeout;

fn with_data() -> MediaOptions {
    MediaOptions {
        udp_addrs: vec!["127.0.0.1:0".into()],
        data_channel: true,
        ..Default::default()
    }
}

async fn wait_for(events: &mut MediaEvents, want: MediaState) {
    timeout(Duration::from_secs(10), async {
        while let Some(state) = events.state.recv().await {
            if state == want {
                return;
            }
        }
        panic!("state channel closed before {want:?}");
    })
    .await
    .unwrap_or_else(|_| panic!("no {want:?} within 10 s"));
}

/// Sends `message` until the other side's data queue yields it (a message sent before the
/// channel opens is not delivered).
async fn deliver(from: &MediaSession, to: &mut MediaEvents, message: &[u8]) -> Vec<u8> {
    let sender = from.data_sender().expect("session has the data channel");
    timeout(Duration::from_secs(15), async {
        loop {
            let _ = sender.send(message).await;
            if let Ok(Some(got)) = timeout(Duration::from_millis(500), to.data.recv()).await {
                return got;
            }
        }
    })
    .await
    .expect("no data-channel message within 15 s")
}

/// Our offer as the stand-in media server reads it: `a=setup:active` makes webrtc-rs the DTLS
/// server there (it takes the inverse of an explicit remote role).
fn as_dtls_server_sees(offer: &str) -> String {
    offer.replace("a=setup:actpass", "a=setup:active")
}

/// The stand-in's answer as the media server writes it: `a=setup:passive`, as the live
/// meeting's answer was (webrtc-rs writes its default `active` in the text whatever role it
/// takes).
fn as_media_server_answers(answer: &str) -> String {
    answer.replace("a=setup:active", "a=setup:passive")
}

/// Each side opens its own `main-channel`, so each reads one channel it opened and one the other
/// side opened (`on_data_channel`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_channel_opens_when_we_are_the_dtls_client() {
    let (ours, mut our_events) = MediaSession::with_options(vec![], with_data())
        .await
        .unwrap();
    let (server, mut server_events) = MediaSession::with_options(vec![], with_data())
        .await
        .unwrap();

    let offer = ours.create_offer().await.unwrap();
    let answer = server
        .accept_offer(&as_dtls_server_sees(&offer))
        .await
        .unwrap();
    let answer = as_media_server_answers(&answer);
    ours.set_answer(&answer).await.unwrap();

    wait_for(&mut our_events, MediaState::Connected).await;
    wait_for(&mut server_events, MediaState::Connected).await;

    assert_eq!(
        deliver(&ours, &mut server_events, b"to the server").await,
        b"to the server"
    );
    assert_eq!(deliver(&server, &mut our_events, b"to us").await, b"to us");
}
