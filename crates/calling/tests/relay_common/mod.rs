//! A fake TURN relay on loopback for the relay tests.

use calling::relay::RelayCredentials;
use rtc::stun::addr::AlternateServer;
use rtc::stun::attributes::{ATTR_ALTERNATE_SERVER, ATTR_NONCE, ATTR_REALM, ATTR_USERNAME};
use rtc::stun::error_code::{ErrorCode, ErrorCodeAttribute};
use rtc::stun::message::{
    CLASS_ERROR_RESPONSE, CLASS_SUCCESS_RESPONSE, METHOD_ALLOCATE, Message, MessageType, Setter,
};
use rtc::stun::textattrs::{Nonce, Realm, Username};
use rtc::turn::proto::lifetime::Lifetime;
use rtc::turn::proto::relayaddr::RelayedAddress;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::UdpSocket;

pub fn creds() -> RelayCredentials {
    RelayCredentials {
        realm: "rtcmedia".into(),
        username: "relay-user".into(),
        password: "relay-pass".into(),
    }
}

fn reply(
    req: &Message,
    class: rtc::stun::message::MessageClass,
    attrs: Vec<Box<dyn Setter>>,
) -> Message {
    let mut m = Message::new();
    let mut all: Vec<Box<dyn Setter>> = vec![
        Box::new(req.transaction_id),
        Box::new(MessageType::new(req.typ.method, class)),
    ];
    all.extend(attrs);
    m.build(&all).unwrap();
    m
}

fn code(c: u16) -> Box<dyn Setter> {
    Box::new(ErrorCodeAttribute {
        code: ErrorCode(c),
        reason: b"x".to_vec(),
    })
}

/// A fake relay: redirects every authenticated Allocate to `alternate` when one is given,
/// otherwise allocates. Unauthenticated requests get a 401. `drop_first` datagrams are ignored
/// (to exercise retransmission).
pub async fn fake_relay(alternate: Option<SocketAddr>, mut drop_first: usize) -> SocketAddr {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 1500];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                return;
            };
            if drop_first > 0 {
                drop_first -= 1;
                continue;
            }
            let mut req = Message::new();
            if req.unmarshal_binary(&buf[..n]).is_err() || req.typ.method != METHOD_ALLOCATE {
                continue;
            }
            let authed = Username::get_from_as(&req, ATTR_USERNAME).is_ok();
            let answer = match (authed, alternate) {
                (false, _) => reply(
                    &req,
                    CLASS_ERROR_RESPONSE,
                    vec![
                        code(401),
                        Box::new(Realm::new(ATTR_REALM, "rtcmedia".into())),
                        Box::new(Nonce::new(ATTR_NONCE, "nonce".into())),
                    ],
                ),
                (true, Some(alt)) => {
                    let mut m = reply(&req, CLASS_ERROR_RESPONSE, vec![code(300)]);
                    AlternateServer {
                        ip: alt.ip(),
                        port: alt.port(),
                    }
                    .add_to_as(&mut m, ATTR_ALTERNATE_SERVER)
                    .unwrap();
                    m
                }
                (true, None) => reply(
                    &req,
                    CLASS_SUCCESS_RESPONSE,
                    vec![
                        Box::new(RelayedAddress {
                            ip: local.ip(),
                            port: 9,
                        }),
                        Box::new(Lifetime(Duration::from_secs(600))),
                    ],
                ),
            };
            let _ = sock.send_to(&answer.raw, from).await;
        }
    });
    local
}
