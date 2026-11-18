//! Two `MediaSession`s on localhost (no TURN, no network) negotiate through the Teams SDP
//! dialect, connect over ICE + DTLS-SRTP, and carry Opus packets both ways.

use std::time::Duration;

use calling::OpusPacket;
use calling::media::{MediaEvents, MediaOptions, MediaSession, MediaState};
use tokio::time::timeout;

fn local() -> MediaOptions {
    MediaOptions {
        udp_addrs: vec!["127.0.0.1:0".into()],
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

/// A fake 20 ms Opus packet: the TOC byte of a CELT-only 20 ms mono frame, then a counter.
fn packet(i: u8) -> OpusPacket {
    vec![0xfc, i, i, i, i, i, i, i]
}

async fn pump(session: &MediaSession, events: &mut MediaEvents, tag: u8) -> Vec<OpusPacket> {
    let sender = session.audio_sender();
    let send = async move {
        for i in 0..50u8 {
            sender.send(packet(i ^ tag)).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let mut got = Vec::new();
    let recv = async {
        while got.len() < 20 {
            match timeout(Duration::from_secs(5), events.audio.recv()).await {
                Ok(Some(p)) => got.push(p),
                _ => break,
            }
        }
    };
    tokio::join!(send, recv);
    got
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_carries_opus_both_ways() {
    let (caller, mut caller_events) = MediaSession::with_options(vec![], local()).await.unwrap();
    let (callee, mut callee_events) = MediaSession::with_options(vec![], local()).await.unwrap();

    let offer = caller.create_offer().await.unwrap();
    let answer = callee.accept_offer(&offer).await.unwrap();
    caller.set_answer(&answer).await.unwrap();

    wait_for(&mut caller_events, MediaState::Connected).await;
    wait_for(&mut callee_events, MediaState::Connected).await;

    let (to_callee, to_caller) = tokio::join!(
        pump(&caller, &mut callee_events, 0x00),
        pump(&callee, &mut caller_events, 0x80),
    );
    assert!(to_callee.len() >= 20, "callee got {}", to_callee.len());
    assert!(to_caller.len() >= 20, "caller got {}", to_caller.len());
    // Payloads arrive unchanged, in order.
    assert_eq!(to_callee[0][0], 0xfc);
    assert!(
        to_callee
            .windows(2)
            .all(|w| w[1][1] == w[0][1].wrapping_add(1))
    );
    assert!(to_caller.iter().all(|p| p[1] & 0x80 == 0x80));

    caller.close().await.unwrap();
    wait_for(&mut caller_events, MediaState::Closed).await;
    callee.close().await.unwrap();
}
