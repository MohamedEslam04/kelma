//! The TURN-over-TLS shim and probe, offline: the relay is a TLS server on loopback with a
//! certificate from a test root, either echoing frames or bridging them to the UDP fake relay.

mod relay_common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use calling::IceServer;
use calling::media::{MediaOptions, MediaSession};
use calling::relay::tls::{
    StreamDeframer, TlsRelay, TlsShim, client_config, find_relay_tls, frame_for_stream,
};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
use relay_common::{creds, fake_relay};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore, ServerConfig};

const NAME: &str = "relay.test";

/// A test root, and a server config for `NAME` signed by it.
struct TestPki {
    root: CertificateDer<'static>,
    server: Arc<ServerConfig>,
}

fn pki() -> TestPki {
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(ca, KeyPair::generate().unwrap()).unwrap();
    let leaf_key = KeyPair::generate().unwrap();
    let leaf = CertificateParams::new(vec![NAME.to_owned()])
        .unwrap()
        .signed_by(&leaf_key, &ca)
        .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        )
        .unwrap();
    TestPki {
        root: ca.der().clone(),
        server: Arc::new(server),
    }
}

/// A client config trusting only the test root.
fn trusting(root: &CertificateDer<'static>) -> Arc<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(root.clone()).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// What a test relay does with each frame it reads.
#[derive(Clone, Copy)]
enum Behaviour {
    /// Write the frame back.
    Echo,
    /// Pass it to this UDP TURN server and frame its answers back.
    Bridge(SocketAddr),
    /// Close the stream at once.
    Hang,
}

/// A TLS relay on loopback. Returns its address and a count of accepted streams.
async fn tls_relay(
    server: Arc<ServerConfig>,
    behaviour: Behaviour,
) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    let acceptor = TlsAcceptor::from(server);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            let count = Arc::clone(&count);
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                count.fetch_add(1, Ordering::SeqCst);
                let udp = match behaviour {
                    Behaviour::Hang => return,
                    Behaviour::Echo => None,
                    Behaviour::Bridge(turn) => {
                        let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                        s.connect(turn).await.unwrap();
                        Some(s)
                    }
                };
                let mut deframer = StreamDeframer::default();
                let mut buf = vec![0u8; 4096];
                let mut answer = vec![0u8; 1500];
                loop {
                    tokio::select! {
                        n = tls.read(&mut buf) => {
                            let Ok(n @ 1..) = n else { return };
                            deframer.push(&buf[..n]);
                            while let Some(frame) = deframer.next_frame().unwrap() {
                                match &udp {
                                    Some(udp) => { udp.send(&frame).await.unwrap(); }
                                    None => tls.write_all(&frame_for_stream(&frame).unwrap()).await.unwrap(),
                                }
                            }
                        }
                        n = async { udp.as_ref().unwrap().recv(&mut answer).await }, if udp.is_some() => {
                            let n = n.unwrap();
                            tls.write_all(&frame_for_stream(&answer[..n]).unwrap()).await.unwrap();
                        }
                    }
                }
            });
        }
    });
    (addr, accepted)
}

fn relay(addr: SocketAddr) -> TlsRelay {
    TlsRelay {
        addr,
        server_name: NAME.to_owned(),
    }
}

fn stun(tid: u8) -> Vec<u8> {
    let mut m = vec![0x00, 0x01, 0x00, 0x04, 0x21, 0x12, 0xA4, 0x42];
    m.extend_from_slice(&[tid; 12]);
    m.extend_from_slice(&[0x80, 0x22, 0x00, 0x00]);
    m
}

fn channel_data(len: usize) -> Vec<u8> {
    let mut m = vec![0x40, 0x01];
    m.extend_from_slice(&(len as u16).to_be_bytes());
    m.extend((0..len).map(|i| i as u8));
    m
}

async fn client() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").await.unwrap()
}

async fn recv(sock: &UdpSocket) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 1500];
    let (n, _) = tokio::time::timeout(Duration::from_secs(2), sock.recv_from(&mut buf))
        .await
        .ok()?
        .ok()?;
    Some(buf[..n].to_vec())
}

#[tokio::test]
async fn datagrams_round_trip_through_the_shim() {
    let pki = pki();
    let (addr, _) = tls_relay(pki.server, Behaviour::Echo).await;
    let shim = TlsShim::start(relay(addr), trusting(&pki.root))
        .await
        .unwrap();
    assert!(shim.local_addr().ip().is_loopback());
    assert_eq!(
        shim.turn_url(),
        format!("turn:{}?transport=udp", shim.local_addr())
    );
    let sock = client().await;
    for d in [stun(1), channel_data(5), channel_data(8)] {
        sock.send_to(&d, shim.local_addr()).await.unwrap();
        assert_eq!(recv(&sock).await.as_deref(), Some(&d[..]));
    }
}

#[tokio::test]
async fn a_retransmitted_request_goes_on_the_stream_once() {
    let pki = pki();
    let (addr, _) = tls_relay(pki.server, Behaviour::Echo).await;
    let shim = TlsShim::start(relay(addr), trusting(&pki.root))
        .await
        .unwrap();
    let sock = client().await;
    sock.send_to(&stun(1), shim.local_addr()).await.unwrap();
    sock.send_to(&stun(1), shim.local_addr()).await.unwrap();
    sock.send_to(&stun(2), shim.local_addr()).await.unwrap();
    assert_eq!(recv(&sock).await, Some(stun(1)));
    assert_eq!(recv(&sock).await, Some(stun(2)));
}

#[tokio::test]
async fn each_client_socket_gets_its_own_stream() {
    let pki = pki();
    let (addr, accepted) = tls_relay(pki.server, Behaviour::Echo).await;
    let shim = TlsShim::start(relay(addr), trusting(&pki.root))
        .await
        .unwrap();
    let (a, b) = (client().await, client().await);
    a.send_to(&stun(1), shim.local_addr()).await.unwrap();
    assert_eq!(recv(&a).await, Some(stun(1)));
    b.send_to(&stun(2), shim.local_addr()).await.unwrap();
    assert_eq!(recv(&b).await, Some(stun(2)));
    assert!(recv(&a).await.is_none(), "b's answer must not reach a");
    // The shim's start-up check, then one stream per client.
    assert_eq!(accepted.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_certificate_for_another_name_is_refused() {
    let pki = pki();
    let (addr, _) = tls_relay(pki.server, Behaviour::Echo).await;
    let other = TlsRelay {
        addr,
        server_name: "other.test".into(),
    };
    assert!(TlsShim::start(other, trusting(&pki.root)).await.is_err());
}

#[tokio::test]
async fn a_certificate_outside_the_web_pki_is_refused() {
    let pki = pki();
    let (addr, _) = tls_relay(pki.server, Behaviour::Echo).await;
    assert!(TlsShim::start(relay(addr), client_config()).await.is_err());
}

#[tokio::test]
async fn a_closed_stream_is_not_reopened_for_the_same_client() {
    let pki = pki();
    let (addr, accepted) = tls_relay(pki.server, Behaviour::Hang).await;
    let shim = TlsShim::start(relay(addr), trusting(&pki.root))
        .await
        .unwrap();
    let sock = client().await;
    sock.send_to(&stun(1), shim.local_addr()).await.unwrap();
    assert!(recv(&sock).await.is_none());
    sock.send_to(&stun(2), shim.local_addr()).await.unwrap();
    assert!(recv(&sock).await.is_none());
    // The shim's start-up check, then the client's one stream.
    assert_eq!(accepted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_probe_allocates_over_tls() {
    let pki = pki();
    let turn = fake_relay(None, 0).await;
    let (addr, _) = tls_relay(pki.server, Behaviour::Bridge(turn)).await;
    let found = find_relay_tls(addr, NAME, &creds(), trusting(&pki.root))
        .await
        .unwrap();
    assert_eq!(found.addr, addr);
    assert_eq!(found.server_name, NAME);
}

#[tokio::test]
async fn the_probe_follows_a_redirect_and_checks_the_alternate_as_the_gateway() {
    let pki = pki();
    let (alternate, _) = tls_relay(
        pki.server.clone(),
        Behaviour::Bridge(fake_relay(None, 0).await),
    )
    .await;
    let (gateway, _) = tls_relay(
        pki.server,
        Behaviour::Bridge(fake_relay(Some(alternate), 0).await),
    )
    .await;
    let found = find_relay_tls(gateway, NAME, &creds(), trusting(&pki.root))
        .await
        .unwrap();
    assert_eq!(found.addr, alternate);
    assert_eq!(found.server_name, NAME);
}

#[tokio::test]
async fn relay_only_media_gathers_a_relay_candidate_through_the_shim() {
    let pki = pki();
    let turn = fake_relay(None, 0).await;
    let (addr, _) = tls_relay(pki.server, Behaviour::Bridge(turn)).await;
    let shim = TlsShim::start(relay(addr), trusting(&pki.root))
        .await
        .unwrap();
    let c = creds();
    let ice = vec![IceServer {
        urls: vec![shim.turn_url()],
        username: Some(c.username),
        credential: Some(c.password),
    }];
    let options = MediaOptions {
        udp_addrs: vec!["127.0.0.1:0".into()],
        relay_only: true,
        ..Default::default()
    };
    let (session, _events) = MediaSession::with_options(ice, options).await.unwrap();
    let offer = session.create_offer().await.unwrap();
    let candidates: Vec<&str> = offer
        .split("\r\n")
        .filter(|l| l.starts_with("a=candidate:"))
        .collect();
    assert!(!candidates.is_empty(), "{offer}");
    assert!(
        candidates.iter().all(|c| c.contains(" typ relay")),
        "{offer}"
    );
    session.close().await.unwrap();
}
