//! A TURN Allocate probe that follows `300 Try Alternate`.
//!
//! The Teams relay gateways (`gateway-<geo>.az.relay.teams.cloud.microsoft`) answer the first,
//! unauthenticated Allocate with `300 Try Alternate`, an `ALTERNATE-SERVER` (RFC 5389 §11,
//! RFC 8656 §9) and also a REALM and NONCE; the alternate answers 401 as usual (seen live,
//! 2026-11-18, one hop). webrtc-rs 0.21.1 takes that 300 for a challenge, signs its retry to the
//! gateway and fails on the second 300. So before the peer connection is built, [`find_relay`]
//! walks the redirects itself (a few hops at most) and returns the address that really allocates;
//! the ICE server then names that address. [`AllocateProbe`] is the message logic, free of I/O;
//! [`find_relay`] moves its datagrams over UDP. The probe's own allocation is released at once
//! (Refresh with `LIFETIME` 0).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use rtc::crypto::default_provider;
use rtc::stun::addr::AlternateServer;
use rtc::stun::attributes::{ATTR_ALTERNATE_SERVER, ATTR_NONCE, ATTR_REALM, ATTR_USERNAME};
use rtc::stun::error_code::ErrorCodeAttribute;
use rtc::stun::fingerprint::FINGERPRINT;
use rtc::stun::integrity::MessageIntegrity;
use rtc::stun::message::{
    CLASS_ERROR_RESPONSE, CLASS_REQUEST, CLASS_SUCCESS_RESPONSE, Getter, METHOD_ALLOCATE,
    METHOD_REFRESH, Message, MessageType, Method, Setter, TransactionId,
};
use rtc::stun::textattrs::{Nonce, Realm, Username};
use rtc::turn::proto::PROTO_UDP;
use rtc::turn::proto::lifetime::Lifetime;
use rtc::turn::proto::reqtrans::RequestedTransport;
use tokio::net::UdpSocket;

use super::RelayCredentials;
use crate::error::{Error, Result};

/// How many `300 Try Alternate` answers are followed before giving up.
pub const MAX_REDIRECTS: usize = 3;
/// 401/438 challenges accepted from one server: the first, and one fresh nonce.
const MAX_CHALLENGES: u8 = 2;
/// First retransmission timeout; it doubles on each of [`ATTEMPTS`] sends (0.5 + 1 + 2 s).
const RTO: Duration = Duration::from_millis(500);
const ATTEMPTS: u32 = 3;

/// What to do next.
pub enum ProbeStep {
    /// Send `datagram` to `to` and feed the answer to [`AllocateProbe::on_answer`].
    Send { to: SocketAddr, datagram: Vec<u8> },
    /// `server` allocated: it is the relay. `release` (send it to `server`, no answer needed)
    /// deletes the probe's allocation.
    Found {
        server: SocketAddr,
        release: Vec<u8>,
    },
}

impl std::fmt::Debug for ProbeStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The datagrams carry the username in clear.
        match self {
            Self::Send { to, datagram } => write!(f, "Send({to}, {} bytes)", datagram.len()),
            Self::Found { server, .. } => write!(f, "Found({server})"),
        }
    }
}

/// The Allocate exchange with a relay and the alternates it names, without I/O.
pub struct AllocateProbe {
    username: String,
    password: String,
    server: SocketAddr,
    /// Every server asked so far, the first included.
    visited: Vec<SocketAddr>,
    /// Realm and nonce from the current server's challenge.
    auth: Option<(String, String)>,
    challenges: u8,
    pending: Option<TransactionId>,
}

impl std::fmt::Debug for AllocateProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllocateProbe")
            .field("server", &self.server)
            .field("visited", &self.visited)
            .finish_non_exhaustive()
    }
}

fn stun_err(context: &str) -> impl FnOnce(rtc::shared::error::Error) -> Error + '_ {
    move |e| Error::Protocol(format!("TURN {context}: {e}"))
}

impl AllocateProbe {
    pub fn new(server: SocketAddr, credentials: &RelayCredentials) -> Self {
        Self {
            username: credentials.username.clone(),
            password: credentials.password.clone(),
            server,
            visited: vec![server],
            auth: None,
            challenges: 0,
            pending: None,
        }
    }

    /// The first, unauthenticated Allocate.
    pub fn start(&mut self) -> Result<ProbeStep> {
        self.allocate()
    }

    /// Reads one datagram. `Ok(None)` when it is not the answer to the request in flight.
    pub fn on_answer(&mut self, datagram: &[u8]) -> Result<Option<ProbeStep>> {
        let mut m = Message::new();
        if m.unmarshal_binary(datagram).is_err()
            || self.pending != Some(m.transaction_id)
            || m.typ.method != METHOD_ALLOCATE
        {
            return Ok(None);
        }
        let server = self.server;
        if m.typ.class == CLASS_SUCCESS_RESPONSE {
            self.pending = None;
            let release = self.request(METHOD_REFRESH, Some(Duration::ZERO))?.1;
            return Ok(Some(ProbeStep::Found { server, release }));
        }
        if m.typ.class != CLASS_ERROR_RESPONSE {
            return Ok(None);
        }
        let mut code = ErrorCodeAttribute::default();
        code.get_from(&m).map_err(stun_err("error answer"))?;
        match code.code.0 {
            // Unauthorized, Stale Nonce: answer the challenge.
            401 | 438 => {
                self.challenges += 1;
                if self.challenges > MAX_CHALLENGES {
                    return Err(Error::Protocol(format!(
                        "TURN relay {server} rejected the credentials ({code})"
                    )));
                }
                let realm = Realm::get_from_as(&m, ATTR_REALM).map_err(stun_err("challenge"))?;
                let nonce = Nonce::get_from_as(&m, ATTR_NONCE).map_err(stun_err("challenge"))?;
                self.auth = Some((realm.text, nonce.text));
                self.allocate().map(Some)
            }
            300 => {
                let mut alternate = AlternateServer::default();
                alternate
                    .get_from_as(&m, ATTR_ALTERNATE_SERVER)
                    .map_err(stun_err("300 without ALTERNATE-SERVER"))?;
                let alternate = SocketAddr::new(alternate.ip, alternate.port);
                if self.visited.contains(&alternate) {
                    return Err(Error::Protocol(format!(
                        "TURN relay {server} redirects back to {alternate}"
                    )));
                }
                if self.visited.len() > MAX_REDIRECTS {
                    return Err(Error::Protocol(format!(
                        "TURN relay still redirecting after {MAX_REDIRECTS} hops"
                    )));
                }
                self.visited.push(alternate);
                self.server = alternate;
                self.auth = None;
                self.challenges = 0;
                self.allocate().map(Some)
            }
            _ => Err(Error::Protocol(format!(
                "TURN relay {server} refused Allocate: {code}"
            ))),
        }
    }

    fn allocate(&mut self) -> Result<ProbeStep> {
        let (tid, datagram) = self.request(METHOD_ALLOCATE, None)?;
        self.pending = Some(tid);
        Ok(ProbeStep::Send {
            to: self.server,
            datagram,
        })
    }

    /// One request to the current server, signed when a challenge was answered.
    fn request(
        &self,
        method: Method,
        lifetime: Option<Duration>,
    ) -> Result<(TransactionId, Vec<u8>)> {
        let provider =
            default_provider().map_err(|e| Error::Protocol(format!("TURN crypto: {e}")))?;
        let tid = TransactionId::new();
        let mut attrs: Vec<Box<dyn Setter + '_>> = vec![
            Box::new(tid),
            Box::new(MessageType::new(method, CLASS_REQUEST)),
        ];
        if method == METHOD_ALLOCATE {
            attrs.push(Box::new(RequestedTransport {
                protocol: PROTO_UDP,
            }));
        }
        if let Some(lifetime) = lifetime {
            attrs.push(Box::new(Lifetime(lifetime)));
        }
        if let Some((realm, nonce)) = &self.auth {
            attrs.push(Box::new(Username::new(
                ATTR_USERNAME,
                self.username.clone(),
            )));
            attrs.push(Box::new(Realm::new(ATTR_REALM, realm.clone())));
            attrs.push(Box::new(Nonce::new(ATTR_NONCE, nonce.clone())));
            attrs.push(Box::new(
                MessageIntegrity::new_long_term_integrity_with_provider(
                    self.username.clone(),
                    realm.clone(),
                    self.password.clone(),
                    provider.crypto(),
                )
                .map_err(stun_err("integrity"))?,
            ));
        }
        attrs.push(Box::new(FINGERPRINT));
        let mut m = Message::new();
        m.build(&attrs).map_err(stun_err("request"))?;
        Ok((tid, m.raw))
    }
}

/// Asks `server` for an allocation over UDP, following its redirects, and returns the address
/// that allocated. The probe's allocation is released before this returns.
pub async fn find_relay(server: SocketAddr, credentials: &RelayCredentials) -> Result<SocketAddr> {
    let bind = if server.is_ipv4() {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
    } else {
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| Error::Protocol(format!("TURN probe socket: {e}")))?;
    let mut probe = AllocateProbe::new(server, credentials);
    let mut step = probe.start()?;
    let mut buf = vec![0u8; 1500];
    loop {
        let (to, datagram) = match step {
            ProbeStep::Found { server, release } => {
                let _ = socket.send_to(&release, server).await;
                return Ok(server);
            }
            ProbeStep::Send { to, datagram } => {
                tracing::debug!(%to, "TURN probe: Allocate");
                (to, datagram)
            }
        };
        step = 'answered: {
            for attempt in 0..ATTEMPTS {
                socket
                    .send_to(&datagram, to)
                    .await
                    .map_err(|e| Error::Protocol(format!("TURN probe to {to}: {e}")))?;
                let deadline = tokio::time::Instant::now() + RTO * 2u32.pow(attempt);
                while let Ok(received) =
                    tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await
                {
                    // An ICMP-refused port surfaces here as an error: treat it as silence.
                    let Ok((n, from)) = received else { break };
                    if from != to {
                        continue;
                    }
                    if let Some(next) = probe.on_answer(&buf[..n])? {
                        break 'answered next;
                    }
                }
            }
            return Err(Error::Protocol(format!("TURN relay {to} did not answer")));
        };
    }
}
