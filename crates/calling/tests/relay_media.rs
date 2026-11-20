//! Relay-only ICE: a `MediaSession` that offers relayed candidates and nothing else (offline:
//! the relay is a fake on loopback).

mod relay_common;

use calling::IceServer;
use calling::media::{MediaOptions, MediaSession};
use relay_common::{creds, fake_relay};

fn relay_only() -> MediaOptions {
    MediaOptions {
        udp_addrs: vec!["127.0.0.1:0".into()],
        relay_only: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn relay_only_without_a_turn_server_is_refused() {
    assert!(
        MediaSession::with_options(vec![], relay_only())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn relay_only_offers_no_host_candidates() {
    let relay = fake_relay(None, 0).await;
    let c = creds();
    let ice = vec![IceServer {
        urls: vec![format!("turn:{relay}?transport=udp")],
        username: Some(c.username),
        credential: Some(c.password),
    }];
    let (session, _events) = MediaSession::with_options(ice, relay_only()).await.unwrap();
    let offer = session.create_offer().await.unwrap();
    let candidates: Vec<&str> = offer
        .split("\r\n")
        .filter(|l| l.starts_with("a=candidate:"))
        .collect();
    assert!(
        candidates.iter().all(|c| c.contains(" typ relay")),
        "{offer}"
    );
    assert!(!candidates.is_empty(), "{offer}");
    session.close().await.unwrap();
}

#[test]
fn relay_only_is_off_by_default() {
    assert!(!MediaOptions::default().relay_only);
}
