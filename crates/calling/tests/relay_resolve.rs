//! Picking the relay host, probing it over UDP, and the ICE servers built from the answer
//! (offline: the "relays" are UDP sockets on loopback).

mod relay_common;

use calling::relay::allocate::find_relay;
use calling::relay::{RelayGrant, relay_hosts, resolved_ice_servers};
use relay_common::{creds, fake_relay};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;

#[test]
fn relay_hosts_start_with_the_users_geo() {
    let eu = "gateway-eu.az.relay.teams.cloud.microsoft";
    let us = "gateway-us.az.relay.teams.cloud.microsoft";
    let world = "gateway-world.az.relay.teams.cloud.microsoft";
    assert_eq!(relay_hosts("amer"), vec![us, world, eu]);
    assert_eq!(relay_hosts("emea"), vec![eu, world]);
    assert_eq!(relay_hosts("EMEA"), vec![eu, world]);
    assert_eq!(
        relay_hosts("in"),
        vec!["gateway-in.az.relay.teams.cloud.microsoft", world, eu]
    );
    // Unknown or missing region: the world gateway, then the EU default.
    assert_eq!(relay_hosts("apac"), vec![world, eu]);
    assert_eq!(relay_hosts(""), vec![world, eu]);
}

#[test]
fn resolved_ice_servers_name_the_relay_by_address() {
    let grant = RelayGrant {
        credentials: vec![creds()],
        expires_in: Duration::from_secs(60),
    };
    let v4: SocketAddr = "198.51.100.7:3478".parse().unwrap();
    let s = resolved_ice_servers(&grant, v4);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].urls, vec!["turn:198.51.100.7:3478?transport=udp"]);
    assert_eq!(s[0].username.as_deref(), Some("relay-user"));
    assert_eq!(s[0].credential.as_deref(), Some("relay-pass"));
    let v6: SocketAddr = "[2001:db8::7]:3478".parse().unwrap();
    assert_eq!(
        resolved_ice_servers(&grant, v6)[0].urls,
        vec!["turn:[2001:db8::7]:3478?transport=udp"]
    );
}

#[tokio::test]
async fn find_relay_follows_the_gateway_to_the_relay() {
    let relay = fake_relay(None, 1).await;
    let gateway = fake_relay(Some(relay), 0).await;
    let found = tokio::time::timeout(Duration::from_secs(10), find_relay(gateway, &creds()))
        .await
        .expect("bounded")
        .unwrap();
    assert_eq!(found, relay);
}

#[tokio::test]
async fn find_relay_gives_up_on_a_silent_server() {
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = silent.local_addr().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(15), find_relay(addr, &creds()))
        .await
        .expect("find_relay must time out on its own");
    assert!(result.is_err());
}
