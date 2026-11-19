//! One WebRTC peer (webrtc-rs) carrying a call's audio: Opus over ICE/TURN and DTLS-SRTP.
//!
//! [`MediaSession`] wraps one peer connection with a single sendrecv audio transceiver. ICE is
//! not trickled: the NGC service carries one SDP blob per direction, so [`MediaSession::create_offer`]
//! and [`MediaSession::accept_offer`] wait for candidate gathering to finish before they return
//! the SDP. Every SDP crosses [`sdp`]'s rewrites on the way out and in, so the strings this module
//! takes and returns are in Teams' dialect, ready for (and straight from) an [`crate::SdpBlob`].
//!
//! Keying is DTLS-SRTP only, as the Teams web client (a browser) does it; webrtc-rs has no
//! SDES. Audio is Opus 48 kHz/2 at payload type 111 with `minptime=10;useinbandfec=1`, as the
//! web client offers it (teams-lite `NATIVE-CALLING.md` §2.5); an offer naming Opus under
//! another payload type (a desktop client's 102) is answered with that one. Encoding and decoding
//! belong to [`crate::audio`]; this module moves [`OpusPacket`]s.

pub mod sdp;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use rtc::ice::mdns::MulticastDnsMode;
use rtc::interceptor::Registry;
use rtc::media::Sample;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::MIME_TYPE_OPUS;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
    RtpCodecKind,
};
use tokio::sync::{OnceCell, mpsc, watch};
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCIceGatheringState, RTCIceServer, RTCIceTransportPolicy,
    RTCPeerConnectionState, RTCSessionDescription, SettingEngineBuilder,
};
use webrtc::rtp_transceiver::RtpSender;

use crate::{Error, IceServer, OpusPacket, Result};

/// The Opus payload type the web client offers.
pub const OPUS_PAYLOAD_TYPE: u8 = 111;
const OPUS_FMTP: &str = "minptime=10;useinbandfec=1";
const FRAME: Duration = Duration::from_millis(20);
/// How long an offer or answer waits for ICE gathering (a TURN allocation that never answers must
/// not hold up a ringing call); after it, the candidates gathered so far go out.
const GATHER_TIMEOUT: Duration = Duration::from_secs(10);
/// Incoming packets buffered for the audio module: 5 s of 20 ms frames.
const AUDIO_QUEUE: usize = 250;

/// Where the peer binds its UDP sockets, and which candidates it may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaOptions {
    /// Local UDP addresses; a wildcard (`0.0.0.0:0`, `[::]:0`) means one socket per interface,
    /// loopback and link-local excluded. Tests bind `127.0.0.1:0`.
    pub udp_addrs: Vec<String>,
    /// Gather and use relayed (TURN) candidates only (ICE transport policy `relay`), to prove
    /// the relay path. Building the session fails when no TURN server is given.
    pub relay_only: bool,
}

impl Default for MediaOptions {
    fn default() -> Self {
        Self {
            udp_addrs: vec!["0.0.0.0:0".to_owned()],
            relay_only: false,
        }
    }
}

/// The peer connection's state, as RTCPeerConnection names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

/// What a session reports: its state changes and the Opus packets the other side sends.
pub struct MediaEvents {
    pub state: mpsc::UnboundedReceiver<MediaState>,
    /// The payload of each incoming Opus RTP packet, in arrival order. Packets are dropped,
    /// not queued without bound, when the reader falls behind.
    pub audio: mpsc::Receiver<OpusPacket>,
    /// Who is heard right now: the CSRCs of the latest audio packet (the meeting mixer names each
    /// speaker's `sourceId`), or its SSRC when it names none (one sender). Updated on change only.
    pub speakers: watch::Receiver<Vec<u32>>,
}

/// One call's media peer.
pub struct MediaSession {
    pc: Arc<dyn PeerConnection>,
    sender: AudioSender,
    gathered: watch::Receiver<bool>,
    state: Arc<StateTx>,
}

/// Sends our Opus packets, one 20 ms frame each. Cheap to clone into the audio task.
#[derive(Clone)]
pub struct AudioSender {
    track: Arc<TrackLocalStaticSample>,
    rtp_sender: Arc<dyn RtpSender>,
    ssrc: u32,
    payload_type: Arc<OnceCell<u8>>,
}

fn media_err(context: &str) -> impl FnOnce(webrtc::error::Error) -> Error + '_ {
    move |e| Error::Media(format!("{context}: {e}"))
}

impl MediaSession {
    /// A peer using `ice_servers` (TURN from `trap/tokens`), bound on every interface.
    pub async fn new(ice_servers: Vec<IceServer>) -> Result<(Self, MediaEvents)> {
        Self::with_options(ice_servers, MediaOptions::default()).await
    }

    pub async fn with_options(
        ice_servers: Vec<IceServer>,
        options: MediaOptions,
    ) -> Result<(Self, MediaEvents)> {
        let opus = RTCRtpCodec {
            mime_type: MIME_TYPE_OPUS.to_owned(),
            clock_rate: crate::SAMPLE_RATE,
            channels: 2,
            sdp_fmtp_line: OPUS_FMTP.to_owned(),
            rtcp_feedback: vec![],
        };
        let mut media_engine = MediaEngine::default();
        media_engine
            .register_codec(
                RTCRtpCodecParameters {
                    rtp_codec: opus.clone(),
                    payload_type: OPUS_PAYLOAD_TYPE,
                },
                RtpCodecKind::Audio,
            )
            .map_err(media_err("register Opus"))?;
        let registry = register_default_interceptors(Registry::new(), &mut media_engine)
            .map_err(media_err("interceptors"))?;
        let configuration = RTCConfigurationBuilder::new()
            .with_ice_servers(ice_servers.into_iter().map(ice_server).collect())
            .with_ice_transport_policy(if options.relay_only {
                RTCIceTransportPolicy::Relay
            } else {
                RTCIceTransportPolicy::All
            })
            .build();
        let settings = SettingEngineBuilder::new()
            // Teams cannot resolve `.local` names, and reads a fingerprint per section.
            .with_multicast_dns_mode(MulticastDnsMode::Disabled)
            .with_sdp_media_level_fingerprints(true)
            .build();

        let (state_tx, state_rx) = mpsc::unbounded_channel();
        let state = Arc::new(StateTx {
            tx: state_tx,
            closed: AtomicBool::new(false),
        });
        let (audio_tx, audio_rx) = mpsc::channel(AUDIO_QUEUE);
        let (gathered_tx, gathered) = watch::channel(false);
        let (speakers_tx, speakers_rx) = watch::channel(Vec::new());
        let handler = Handler {
            state: Arc::clone(&state),
            audio: audio_tx,
            gathered: gathered_tx,
            speakers: Arc::new(speakers_tx),
        };
        let pc = PeerConnectionBuilder::new()
            .with_configuration(configuration)
            .with_media_engine(media_engine)
            .with_setting_engine(settings)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(handler))
            .with_udp_addrs(options.udp_addrs)
            .build()
            .await
            .map_err(media_err("build peer connection"))?;
        let pc: Arc<dyn PeerConnection> = Arc::new(pc);

        let ssrc = rand::random::<u32>();
        let track = Arc::new(
            TrackLocalStaticSample::new(
                Instant::now(),
                MediaStreamTrack::new(
                    format!("kelma-{ssrc:08x}"),
                    "kelma-audio".to_owned(),
                    "main-audio".to_owned(),
                    RtpCodecKind::Audio,
                    vec![RTCRtpEncodingParameters {
                        rtp_coding_parameters: RTCRtpCodingParameters {
                            ssrc: Some(ssrc),
                            ..Default::default()
                        },
                        codec: opus,
                        ..Default::default()
                    }],
                ),
            )
            .map_err(media_err("audio track"))?,
        );
        let rtp_sender = pc
            .add_track(Arc::clone(&track) as Arc<dyn TrackLocal>)
            .await
            .map_err(media_err("add audio track"))?;
        let sender = AudioSender {
            track,
            rtp_sender,
            ssrc,
            payload_type: Arc::new(OnceCell::new()),
        };
        Ok((
            Self {
                pc,
                sender,
                gathered,
                state,
            },
            MediaEvents {
                state: state_rx,
                audio: audio_rx,
                speakers: speakers_rx,
            },
        ))
    }

    /// Our offer (for an outgoing call or a meeting join), with every candidate in it.
    pub async fn create_offer(&self) -> Result<String> {
        let offer = self
            .pc
            .create_offer(None)
            .await
            .map_err(media_err("create offer"))?;
        let local = self.set_local(offer).await?;
        Ok(sdp::to_teams_offer(&local))
    }

    /// Takes the other side's answer to [`Self::create_offer`].
    pub async fn set_answer(&self, answer: &str) -> Result<()> {
        let desc = RTCSessionDescription::answer(sdp::from_teams(answer)?)
            .map_err(media_err("parse answer"))?;
        self.pc
            .set_remote_description(desc)
            .await
            .map_err(media_err("set answer"))
    }

    /// Whether `offer` starts a new media session rather than renegotiating this one: its audio
    /// section's mid or ICE ufrag differs from the remote description in force. The media
    /// servers do this when a lobby participant is admitted (audio mid 0 -> 1, new credentials
    /// per kind); answer such an offer on a fresh [`MediaSession`]. `false` before anything was
    /// negotiated, and for an offer with no audio section.
    pub async fn offer_restarts(&self, offer: &str) -> bool {
        let Some(current) = self.pc.remote_description().await else {
            return false;
        };
        match (
            sdp::audio_transport(&current.sdp),
            sdp::audio_transport(offer),
        ) {
            (Some(current), Some(offered)) => current != offered,
            _ => false,
        }
    }

    /// Answers the other side's offer (an incoming call or a renegotiation), with every candidate
    /// in the answer. Refuses an offer that [`Self::offer_restarts`] this session, leaving it as
    /// it was: webrtc-rs would answer its new audio mid with a fresh recvonly transceiver that
    /// carries none of our audio.
    pub async fn accept_offer(&self, offer: &str) -> Result<String> {
        if self.offer_restarts(offer).await {
            return Err(Error::Media(
                "offer restarts the media session (audio mid or ICE ufrag changed); answer it on a new MediaSession".into(),
            ));
        }
        let desc = RTCSessionDescription::offer(sdp::from_teams(offer)?)
            .map_err(media_err("parse offer"))?;
        self.pc
            .set_remote_description(desc)
            .await
            .map_err(media_err("set offer"))?;
        let answer = self
            .pc
            .create_answer(None)
            .await
            .map_err(media_err("create answer"))?;
        let local = self.set_local(answer).await?;
        Ok(sdp::to_teams_answer(&local, offer))
    }

    /// A handle for sending our Opus packets from another task.
    pub fn audio_sender(&self) -> AudioSender {
        self.sender.clone()
    }

    /// Sends one 20 ms Opus packet; see [`AudioSender::send`].
    pub async fn send_audio(&self, packet: OpusPacket) -> Result<()> {
        self.sender.send(packet).await
    }

    /// Ends the session: ICE, DTLS and the sockets are released and the state goes `Closed`.
    pub async fn close(&self) -> Result<()> {
        self.pc.close().await.map_err(media_err("close"))?;
        // webrtc-rs does not report its own close to the handler.
        self.state.send(MediaState::Closed);
        Ok(())
    }

    async fn set_local(&self, desc: RTCSessionDescription) -> Result<String> {
        self.pc
            .set_local_description(desc)
            .await
            .map_err(media_err("set local description"))?;
        let mut gathered = self.gathered.clone();
        if tokio::time::timeout(GATHER_TIMEOUT, gathered.wait_for(|done| *done))
            .await
            .is_err()
        {
            tracing::warn!(
                "ICE gathering still running after {GATHER_TIMEOUT:?}; sending what we have"
            );
        }
        self.pc
            .local_description()
            .await
            .map(|d| d.sdp)
            .ok_or_else(|| Error::Media("no local description".into()))
    }
}

impl AudioSender {
    /// Sends one Opus packet as one 20 ms frame. Before the connection is up the packet goes
    /// nowhere; that is not an error.
    pub async fn send(&self, packet: OpusPacket) -> Result<()> {
        let payload_type = *self
            .payload_type
            .get_or_try_init(|| async {
                let params = self
                    .rtp_sender
                    .get_parameters()
                    .await
                    .map_err(media_err("sender parameters"))?;
                params
                    .rtp_parameters
                    .codecs
                    .first()
                    .map(|c| c.payload_type)
                    .ok_or_else(|| Error::Media("audio is not negotiated yet".into()))
            })
            .await?;
        let sample = Sample {
            data: Bytes::from(packet),
            duration: FRAME,
            ..Sample::new(Instant::now())
        };
        self.track
            .write_sample(self.ssrc, payload_type, &sample, &[])
            .await
            .map_err(media_err("send audio"))
    }
}

fn ice_server(server: IceServer) -> RTCIceServer {
    RTCIceServer {
        urls: server.urls,
        username: server.username.unwrap_or_default(),
        credential: server.credential.unwrap_or_default(),
    }
}

/// State events, with `Closed` sent once whether the peer or [`MediaSession::close`] reports it.
struct StateTx {
    tx: mpsc::UnboundedSender<MediaState>,
    closed: AtomicBool,
}

impl StateTx {
    fn send(&self, state: MediaState) {
        if state == MediaState::Closed && self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(state);
    }
}

/// Who a packet carries: the mixer's CSRC list, or the sender itself.
pub fn heard_sources(csrc: &[u32], ssrc: u32) -> Vec<u32> {
    if csrc.is_empty() {
        vec![ssrc]
    } else {
        csrc.to_vec()
    }
}

struct Handler {
    state: Arc<StateTx>,
    audio: mpsc::Sender<OpusPacket>,
    gathered: watch::Sender<bool>,
    speakers: Arc<watch::Sender<Vec<u32>>>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.send_replace(true);
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let state = match state {
            RTCPeerConnectionState::New => MediaState::New,
            RTCPeerConnectionState::Connecting => MediaState::Connecting,
            RTCPeerConnectionState::Connected => MediaState::Connected,
            RTCPeerConnectionState::Disconnected => MediaState::Disconnected,
            RTCPeerConnectionState::Failed => MediaState::Failed,
            RTCPeerConnectionState::Closed => MediaState::Closed,
            _ => return,
        };
        self.state.send(state);
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        // Teams' SDP names no SSRCs (only `a=x-ssrc-range`), so a remote track can arrive before
        // its kind is known. Only audio is negotiated, so every track is drained as audio; a
        // track nobody polls fills webrtc-rs's queue and its packets are dropped.
        let kind = track.kind().await;
        let id = track.track_id().await.to_string();
        tracing::info!(?kind, track = %id, "remote track");
        if kind == RtpCodecKind::Video {
            // Still drain it, or the driver logs a full queue for every packet.
            tokio::spawn(async move { while track.poll().await.is_some() {} });
            return;
        }
        let audio = self.audio.clone();
        let speakers = Arc::clone(&self.speakers);
        tokio::spawn(async move {
            while let Some(event) = track.poll().await {
                match event {
                    TrackRemoteEvent::OnRtpPacket(packet) if !packet.payload.is_empty() => {
                        let heard = heard_sources(&packet.header.csrc, packet.header.ssrc);
                        speakers.send_if_modified(|now| {
                            let changed = *now != heard;
                            if changed {
                                *now = heard;
                            }
                            changed
                        });
                        // A full queue drops the packet: late audio is worth nothing.
                        if let Err(mpsc::error::TrySendError::Closed(_)) =
                            audio.try_send(packet.payload.to_vec())
                        {
                            break;
                        }
                    }
                    TrackRemoteEvent::OnEnded => break,
                    _ => {}
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::heard_sources;

    #[test]
    fn the_mixer_names_its_speakers_and_a_lone_sender_names_itself() {
        assert_eq!(heard_sources(&[7, 9], 1), [7, 9]);
        assert_eq!(heard_sources(&[], 42), [42]);
    }
}
