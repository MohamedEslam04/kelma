//! The TURN Allocate probe that follows `300 Try Alternate` (offline: every answer is fabricated).

use calling::relay::RelayCredentials;
use calling::relay::allocate::{AllocateProbe, MAX_REDIRECTS, ProbeStep};
use rtc::crypto::default_provider;
use rtc::stun::addr::AlternateServer;
use rtc::stun::attributes::{ATTR_ALTERNATE_SERVER, ATTR_NONCE, ATTR_REALM, ATTR_USERNAME};
use rtc::stun::error_code::{ErrorCode, ErrorCodeAttribute};
use rtc::stun::integrity::MessageIntegrity;
use rtc::stun::message::{
    CLASS_ERROR_RESPONSE, CLASS_REQUEST, CLASS_SUCCESS_RESPONSE, Getter, METHOD_ALLOCATE,
    METHOD_REFRESH, Message, MessageType, Setter,
};
use rtc::stun::textattrs::{Nonce, Realm, Username};
use rtc::stun::xoraddr::XorMappedAddress;
use rtc::turn::proto::lifetime::Lifetime;
use rtc::turn::proto::relayaddr::RelayedAddress;
use std::net::SocketAddr;
use std::time::Duration;

const GATEWAY: &str = "192.0.2.1:3478";
const RELAY_A: &str = "198.51.100.7:3478";
const RELAY_B: &str = "203.0.113.9:3480";

fn creds() -> RelayCredentials {
    RelayCredentials {
        realm: "rtcmedia".into(),
        username: "relay-user".into(),
        password: "relay-pass".into(),
    }
}

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn decode(datagram: &[u8]) -> Message {
    let mut m = Message::new();
    m.unmarshal_binary(datagram).unwrap();
    m
}

/// A request as the server sees it: its type and whether it carries long-term credentials
/// that verify against `relay-user` / `rtcmedia` / `relay-pass`.
fn read_request(step: &ProbeStep) -> (SocketAddr, Message, bool) {
    let ProbeStep::Send { to, datagram } = step else {
        panic!("expected a request, got {step:?}");
    };
    let mut m = decode(datagram);
    assert_eq!(m.typ.class, CLASS_REQUEST);
    let authed = match Username::get_from_as(&m, ATTR_USERNAME) {
        Ok(u) => {
            assert_eq!(u.text, "relay-user");
            assert_eq!(Realm::get_from_as(&m, ATTR_REALM).unwrap().text, "rtcmedia");
            let provider = default_provider().unwrap();
            let key = MessageIntegrity::long_term_integrity_key(
                "relay-user".into(),
                "rtcmedia".into(),
                "relay-pass".into(),
                provider.crypto(),
            )
            .unwrap();
            MessageIntegrity::check(&mut m, &key, provider.crypto()).expect("integrity");
            true
        }
        Err(_) => false,
    };
    (*to, m, authed)
}

fn answer(
    req: &Message,
    class: rtc::stun::message::MessageClass,
    setters: Vec<Box<dyn Setter>>,
) -> Vec<u8> {
    let mut m = Message::new();
    let mut all: Vec<Box<dyn Setter>> = vec![
        Box::new(req.transaction_id),
        Box::new(MessageType::new(req.typ.method, class)),
    ];
    all.extend(setters);
    m.build(&all).unwrap();
    m.raw
}

fn error(code: u16, reason: &str) -> Box<dyn Setter> {
    Box::new(ErrorCodeAttribute {
        code: ErrorCode(code),
        reason: reason.as_bytes().to_vec(),
    })
}

fn challenge(req: &Message, nonce: &str) -> Vec<u8> {
    answer(
        req,
        CLASS_ERROR_RESPONSE,
        vec![
            error(401, "Unauthorized"),
            Box::new(Realm::new(ATTR_REALM, "rtcmedia".into())),
            Box::new(Nonce::new(ATTR_NONCE, nonce.into())),
        ],
    )
}

fn redirect(req: &Message, to: &str) -> Vec<u8> {
    let mut raw = answer(req, CLASS_ERROR_RESPONSE, vec![error(300, "Try Alternate")]);
    let mut m = decode(&raw);
    let to = addr(to);
    AlternateServer {
        ip: to.ip(),
        port: to.port(),
    }
    .add_to_as(&mut m, ATTR_ALTERNATE_SERVER)
    .unwrap();
    raw.clone_from(&m.raw);
    raw
}

fn allocated(req: &Message) -> Vec<u8> {
    answer(
        req,
        CLASS_SUCCESS_RESPONSE,
        vec![
            Box::new(RelayedAddress {
                ip: "198.51.100.7".parse().unwrap(),
                port: 50_000,
            }),
            Box::new(Lifetime(Duration::from_secs(600))),
            Box::new(XorMappedAddress {
                ip: "192.0.2.200".parse().unwrap(),
                port: 40_000,
            }),
        ],
    )
}

#[test]
fn a_gateway_that_allocates_is_the_relay() {
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let first = probe.start().unwrap();
    let (to, req, authed) = read_request(&first);
    assert_eq!((to, authed), (addr(GATEWAY), false));
    assert_eq!(req.typ.method, METHOD_ALLOCATE);

    let second = probe.on_answer(&challenge(&req, "n1")).unwrap().unwrap();
    let (to, req, authed) = read_request(&second);
    assert_eq!((to, authed), (addr(GATEWAY), true));
    assert_eq!(Nonce::get_from_as(&req, ATTR_NONCE).unwrap().text, "n1");

    let done = probe.on_answer(&allocated(&req)).unwrap().unwrap();
    let ProbeStep::Found { server, release } = done else {
        panic!("expected Found, got {done:?}");
    };
    assert_eq!(server, addr(GATEWAY));
    // The probe's own allocation is released: an authenticated Refresh with LIFETIME 0.
    let (to, refresh, authed) = read_request(&ProbeStep::Send {
        to: server,
        datagram: release,
    });
    assert!(authed && to == server);
    assert_eq!(refresh.typ.method, METHOD_REFRESH);
    let mut lifetime = Lifetime::default();
    lifetime.get_from(&refresh).unwrap();
    assert_eq!(lifetime.0, Duration::ZERO);
}

#[test]
fn try_alternate_after_authentication_is_followed() {
    // What the Teams relay does: 401, then 300 to the authenticated Allocate.
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    let (_, req, authed) = read_request(&probe.on_answer(&challenge(&req, "n1")).unwrap().unwrap());
    assert!(authed);
    let next = probe.on_answer(&redirect(&req, RELAY_A)).unwrap().unwrap();
    let (to, req, authed) = read_request(&next);
    // The alternate is asked afresh: its nonce is its own.
    assert_eq!((to, authed), (addr(RELAY_A), false));
    let (to, req, authed) =
        read_request(&probe.on_answer(&challenge(&req, "n2")).unwrap().unwrap());
    assert_eq!((to, authed), (addr(RELAY_A), true));
    assert_eq!(Nonce::get_from_as(&req, ATTR_NONCE).unwrap().text, "n2");
    let done = probe.on_answer(&allocated(&req)).unwrap().unwrap();
    assert!(matches!(done, ProbeStep::Found { server, .. } if server == addr(RELAY_A)));
}

#[test]
fn try_alternate_before_authentication_and_several_hops() {
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    let (to, req, _) = read_request(&probe.on_answer(&redirect(&req, RELAY_A)).unwrap().unwrap());
    assert_eq!(to, addr(RELAY_A));
    let (to, req, _) = read_request(&probe.on_answer(&redirect(&req, RELAY_B)).unwrap().unwrap());
    assert_eq!(to, addr(RELAY_B));
    let (_, req, _) = read_request(&probe.on_answer(&challenge(&req, "n")).unwrap().unwrap());
    let done = probe.on_answer(&allocated(&req)).unwrap().unwrap();
    assert!(matches!(done, ProbeStep::Found { server, .. } if server == addr(RELAY_B)));
}

#[test]
fn redirect_loops_and_endless_redirects_fail() {
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    let (_, req, _) = read_request(&probe.on_answer(&redirect(&req, RELAY_A)).unwrap().unwrap());
    assert!(probe.on_answer(&redirect(&req, GATEWAY)).is_err(), "loop");

    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, mut req, _) = read_request(&probe.start().unwrap());
    let mut result = Ok(None);
    for hop in 0..=MAX_REDIRECTS {
        let next = format!("198.51.100.{}:3478", 100 + hop);
        result = probe.on_answer(&redirect(&req, &next));
        match &result {
            Ok(Some(step)) => req = read_request(step).1,
            _ => break,
        }
    }
    assert!(
        result.is_err(),
        "more than {MAX_REDIRECTS} redirects must fail"
    );
}

#[test]
fn a_refused_allocate_or_rejected_credentials_fail() {
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    let forbidden = answer(&req, CLASS_ERROR_RESPONSE, vec![error(403, "Forbidden")]);
    let err = probe.on_answer(&forbidden).unwrap_err().to_string();
    assert!(err.contains("403"), "{err}");

    // A second 401 to the authenticated request means the credentials are wrong.
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    let (_, req, _) = read_request(&probe.on_answer(&challenge(&req, "n1")).unwrap().unwrap());
    let (_, req, authed) = read_request(&probe.on_answer(&challenge(&req, "n2")).unwrap().unwrap());
    assert!(authed, "a fresh nonce is retried once");
    assert!(probe.on_answer(&challenge(&req, "n3")).is_err());
}

#[test]
fn stray_datagrams_are_ignored() {
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    // Another transaction's answer, and garbage, are not ours.
    let mut other = req.clone();
    other.transaction_id.0[0] ^= 0xff;
    other.write_transaction_id();
    assert!(probe.on_answer(&challenge(&other, "x")).unwrap().is_none());
    assert!(probe.on_answer(b"not stun").unwrap().is_none());
    // The real answer still works.
    assert!(probe.on_answer(&challenge(&req, "n")).unwrap().is_some());
}

#[test]
fn credentials_never_show_in_probe_debug() {
    let probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let text = format!("{probe:?}");
    assert!(
        !text.contains("relay-user") && !text.contains("relay-pass"),
        "{text}"
    );
}

#[test]
fn try_alternate_without_an_alternate_server_fails() {
    // A 300 without ALTERNATE-SERVER cannot be followed.
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    let bare = answer(
        &req,
        CLASS_ERROR_RESPONSE,
        vec![
            error(300, "Try Alternate"),
            Box::new(XorMappedAddress {
                ip: "192.0.2.200".parse().unwrap(),
                port: 1,
            }),
        ],
    );
    assert!(probe.on_answer(&bare).is_err());
}

#[test]
fn the_gateways_unauthenticated_300_with_realm_and_nonce_is_a_redirect() {
    // Live shape (2026-11-18): the gateway answers the very first, unauthenticated Allocate with
    // 300 Try Alternate carrying REALM, NONCE and ALTERNATE-SERVER; the alternate answers 401.
    // webrtc-rs reads that 300 as a challenge and fails on the 300 to its signed retry.
    let mut probe = AllocateProbe::new(addr(GATEWAY), &creds());
    let (_, req, _) = read_request(&probe.start().unwrap());
    let mut m = decode(&redirect(&req, RELAY_A));
    Realm::new(ATTR_REALM, "rtcmedia".into())
        .add_to(&mut m)
        .unwrap();
    Nonce::new(ATTR_NONCE, "gateway-nonce".into())
        .add_to(&mut m)
        .unwrap();
    let (to, _, authed) = read_request(&probe.on_answer(&m.raw).unwrap().unwrap());
    assert_eq!((to, authed), (addr(RELAY_A), false));
}
