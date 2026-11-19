//! Video we send: frames from any source, encoded to H.264, sent on one m-line of the call.
//!
//! The path is the same for every source, so the camera can reuse it: a source produces
//! [`I420Frame`]s (today the ScreenCast portal, [`screencast`]), [`h264::H264Encoder`] turns them
//! into Annex-B access units, and [`crate::media::VideoSender`] packetizes those onto the
//! sending line of a [`VideoRole`]. [`pump`] wires the three together.
//!
//! Teams tells the lines apart by `a=label` only (rusty-teams `sdp.rs`, mautrix-teams
//! `teamsmedia/sdp.go`): `main-video` for the camera (and the gallery), `applicationsharing-video`
//! for a screen share. See `docs/protocol-calls.md` (Screen sharing).

pub mod h264;
#[cfg(feature = "screen-share")]
pub mod screencast;

use crate::{Error, Modality, Result};

/// What a video line carries, as Teams labels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoRole {
    /// The camera: `a=label:main-video`.
    MainVideo,
    /// A screen share: `a=label:applicationsharing-video`.
    ApplicationSharing,
}

impl VideoRole {
    /// The line's `a=label`.
    pub fn label(self) -> &'static str {
        match self {
            VideoRole::MainVideo => "main-video",
            VideoRole::ApplicationSharing => "applicationsharing-video",
        }
    }

    pub fn from_label(label: &str) -> Option<VideoRole> {
        [VideoRole::MainVideo, VideoRole::ApplicationSharing]
            .into_iter()
            .find(|r| r.label() == label)
    }

    /// The `callModalities` entry a renegotiation names while this line sends.
    pub fn modality(self) -> Modality {
        match self {
            VideoRole::MainVideo => Modality::Video,
            VideoRole::ApplicationSharing => Modality::ScreenSharing,
        }
    }

    /// The direction the web client tells the media server for this line while it sends:
    /// `sendrecv` for the camera, `sendonly` for a share (mautrix-teams openapi
    /// `/updateMediaDescriptions`).
    pub fn sending_direction(self) -> &'static str {
        match self {
            VideoRole::MainVideo => "sendrecv",
            VideoRole::ApplicationSharing => "sendonly",
        }
    }

    /// The `negotiationTag` suffix after `<participantId>;`: `ss_1` once a share starts, `v_2`
    /// for the camera on and `v_1` for it off (mautrix-teams openapi).
    pub fn negotiation_suffix(self, sending: bool) -> &'static str {
        match (self, sending) {
            (VideoRole::ApplicationSharing, _) => "ss_1",
            (VideoRole::MainVideo, true) => "v_2",
            (VideoRole::MainVideo, false) => "v_1",
        }
    }
}

/// One raw picture: planar YUV 4:2:0 (I420), tightly packed, even width and height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I420Frame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl I420Frame {
    /// Bytes of one `width` x `height` I420 picture.
    pub fn len_for(width: usize, height: usize) -> usize {
        width * height * 3 / 2
    }

    pub fn new(width: usize, height: usize, data: Vec<u8>) -> Result<Self> {
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(Error::Media(format!(
                "I420 needs an even, non-zero size, not {width}x{height}"
            )));
        }
        if data.len() != Self::len_for(width, height) {
            return Err(Error::Media(format!(
                "a {width}x{height} I420 frame is {} bytes, not {}",
                Self::len_for(width, height),
                data.len()
            )));
        }
        Ok(I420Frame {
            width,
            height,
            data,
        })
    }

    /// The Y, U and V planes.
    pub fn planes(&self) -> (&[u8], &[u8], &[u8]) {
        let y = self.width * self.height;
        let c = y / 4;
        (
            &self.data[..y],
            &self.data[y..y + c],
            &self.data[y + c..y + 2 * c],
        )
    }
}

/// The largest even size within `max` that keeps `source`'s aspect ratio (never upscales). A
/// screen is scaled to fit what the sharing line is allowed (`max-fs` macroblocks).
pub fn fit_within(source: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (w, h) = (source.0.max(2), source.1.max(2));
    let scale = f64::min(
        1.0,
        f64::min(
            f64::from(max.0) / f64::from(w),
            f64::from(max.1) / f64::from(h),
        ),
    );
    let even = |v: f64| ((v as u32) & !1).max(2);
    (even(f64::from(w) * scale), even(f64::from(h) * scale))
}

/// Encodes every frame from `frames` and sends it on `sender` until the source ends, the
/// sender's call goes away or `keyframes` closes. A message on `keyframes` forces the next frame
/// to be a keyframe (a PLI, or `KeyFrame=1` in the media controller's sender control).
#[cfg(feature = "screen-share")]
pub fn pump(
    mut frames: tokio::sync::mpsc::Receiver<I420Frame>,
    mut encoder: h264::H264Encoder,
    sender: crate::media::VideoSender,
    mut keyframes: tokio::sync::mpsc::UnboundedReceiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last = std::time::Instant::now();
        loop {
            let frame = tokio::select! {
                f = frames.recv() => match f { Some(f) => f, None => break },
                k = keyframes.recv() => {
                    if k.is_some() { encoder.request_keyframe(); continue; }
                    break;
                }
            };
            let now = std::time::Instant::now();
            let duration = now - last;
            last = now;
            let encoded = match encoder.encode(&frame) {
                Ok(Some(e)) => e,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(%e, "video frame not encoded");
                    continue;
                }
            };
            if let Err(e) = sender.send(encoded.data, duration).await {
                tracing::debug!(%e, "video frame not sent");
            }
        }
    })
}
