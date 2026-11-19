//! The camera in a live call: the Camera portal (or a V4L2 device), the encoder task feeding the
//! call's `main-video` line, our self-view, and telling the service (see
//! `docs/protocol-calls.md`, Camera video). The other side's video is `remote_video`'s.
//!
//! The main-video line is negotiated only when `KELMA_CAMERA=1` (until a live run confirms the
//! media servers accept it, the verified audio-only offer stays the default), the same opt-in as
//! `KELMA_SCREEN_SHARE=1` for the sharing line. Then:
//! - a meeting flips the line with `UpdateMediaDescriptions` (`sendrecv` with `v_2` on,
//!   `recvonly` with `v_1` off);
//! - a 1:1 call sends a re-offer with `Video` among the `callModalities`.
//!
//! `KELMA_CAMERA_DEVICE=/dev/videoN` picks the V4L2 device used when the portal has no camera;
//! otherwise the camera chosen in Settings > Devices, else `/dev/video0`.

use super::share;
use super::{EventSink, VideoSlot};
use calling::media::MediaSession;
use calling::signaling::video::{
    MediaDescription, negotiation_tag, parse_sender_controls, video_modalities,
};
use calling::signaling::{CallSession, NgcClient};
use calling::video::camera::{CameraCapture, CameraSettings, open};
use calling::video::h264::{EncoderSettings, H264Encoder};
use calling::video::{VideoRole, i420_to_rgb, pump, step_for, tap};
use calling::{Links, SdpBlob};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub const ROLE: VideoRole = VideoRole::MainVideo;
/// The self-view is drawn small, in our own tile: no wider than this.
const SELF_VIEW_WIDTH: usize = 320;
/// One self-view picture every this many camera frames (15 fps camera, 7.5 fps self-view).
const SELF_VIEW_EVERY: u32 = 2;

/// Whether calls negotiate the main-video line (`KELMA_CAMERA` set to anything but `0`).
pub fn wanted() -> bool {
    env_on(std::env::var_os("KELMA_CAMERA").as_deref())
}

fn env_on(v: Option<&std::ffi::OsStr>) -> bool {
    v.is_some_and(|v| !v.is_empty() && v != "0")
}

/// "Accept with video" on the incoming-call toast: the next adopted call starts the camera.
static ACCEPT_WITH_VIDEO: AtomicBool = AtomicBool::new(false);

pub fn accept_with_video(on: bool) {
    ACCEPT_WITH_VIDEO.store(on && wanted(), Ordering::SeqCst);
}

/// Whether the call being adopted was accepted with video (read once).
pub fn take_accept_with_video() -> bool {
    ACCEPT_WITH_VIDEO.swap(false, Ordering::SeqCst)
}

/// The V4L2 device for when the portal has no camera: `KELMA_CAMERA_DEVICE`, else the camera
/// chosen in Settings (remembered by its bus), else the first one found.
fn v4l2_device() -> Option<String> {
    if let Some(d) = std::env::var_os("KELMA_CAMERA_DEVICE").filter(|d| !d.is_empty()) {
        return Some(d.to_string_lossy().into_owned());
    }
    let chosen = session::Paths::new().ok().and_then(|p| {
        crate::backend::settings::Local::load(&p.config_dir)
            .devices
            .camera
    });
    let cameras = devices::cameras().unwrap_or_default();
    device_for(&cameras, chosen.as_deref())
}

/// The device path of the camera on bus `chosen`, else of the first camera.
fn device_for(cameras: &[devices::Camera], chosen: Option<&str>) -> Option<String> {
    chosen
        .and_then(|bus| cameras.iter().find(|c| c.bus == bus))
        .or(cameras.first())
        .map(|c| c.path.clone())
}

/// What the camera tasks report to the call loop.
pub enum CameraMsg {
    /// The camera opened, or why it did not.
    Opened(Result<Box<CameraCapture>, String>),
    /// The capture ended on its own (the camera was unplugged).
    Ended,
}

/// One call's camera state.
#[derive(Default)]
pub struct Camera {
    opening: bool,
    capture: Option<CameraCapture>,
    encoder: Option<JoinHandle<()>>,
    keyframes: Option<mpsc::UnboundedSender<()>>,
    /// `requestId` of the last `UpdateMediaDescriptions`.
    request_id: u64,
    /// What the UI was last told: (available, on).
    shown: Option<(bool, bool)>,
}

impl Camera {
    pub fn on(&self) -> bool {
        self.capture.is_some()
    }

    /// Opens the camera without blocking the call (the portal may ask the user); the result
    /// comes as [`CameraMsg::Opened`].
    pub fn open(&mut self, tx: &mpsc::UnboundedSender<CameraMsg>) {
        if self.opening || self.on() {
            return;
        }
        self.opening = true;
        let tx = tx.clone();
        tokio::spawn(async move {
            let device = tokio::task::spawn_blocking(v4l2_device)
                .await
                .ok()
                .flatten();
            let r = open(device.as_deref(), CameraSettings::default())
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = tx.send(CameraMsg::Opened(r));
        });
    }

    /// Starts sending `capture` on `media`'s main-video line and drawing our self-view. The
    /// service is told by the caller.
    pub fn start(
        &mut self,
        mut capture: CameraCapture,
        media: &MediaSession,
        tx: &mpsc::UnboundedSender<CameraMsg>,
        sink: &EventSink,
    ) -> Result<(), String> {
        self.opening = false;
        let sender = media
            .video_sender(ROLE)
            .ok_or("this call has no video line")?;
        let encoder = H264Encoder::new(EncoderSettings::camera()).map_err(|e| e.to_string())?;
        let view = sink.clone();
        let mut n = 0u32;
        let frames = tap(capture.take_frames(), move |f| {
            n = n.wrapping_add(1);
            if n.is_multiple_of(SELF_VIEW_EVERY) {
                let rgb = i420_to_rgb(f, step_for(f.width, SELF_VIEW_WIDTH), true);
                view.video(VideoSlot::SelfView, Some(rgb));
            }
        });
        let (keys_tx, keys_rx) = mpsc::unbounded_channel();
        let task = pump(frames, encoder, sender, keys_rx);
        let tx = tx.clone();
        let watcher = tokio::spawn(async move {
            let _ = task.await;
            let _ = tx.send(CameraMsg::Ended);
        });
        self.capture = Some(capture);
        self.encoder = Some(watcher);
        self.keyframes = Some(keys_tx);
        Ok(())
    }

    /// Stops the camera and the encoder and clears the self-view. True when it was on.
    pub async fn stop(&mut self, sink: &EventSink) -> bool {
        self.opening = false;
        self.keyframes = None;
        if let Some(e) = self.encoder.take() {
            e.abort();
        }
        match self.capture.take() {
            Some(c) => {
                c.stop().await;
                sink.video(VideoSlot::SelfView, None);
                true
            }
            None => false,
        }
    }

    pub fn open_failed(&mut self) {
        self.opening = false;
    }

    /// A `controlVideoStreaming` callback: a keyframe request reaches the encoder.
    pub fn sender_control(&self, body: &serde_json::Value) {
        if parse_sender_controls(body).iter().any(|c| c.keyframe())
            && let Some(k) = &self.keyframes
        {
            let _ = k.send(());
        }
    }

    pub fn next_request_id(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }

    /// The state to show, when it changed since last shown.
    pub fn changed(&mut self, available: bool) -> Option<(bool, bool)> {
        let now = (available, self.on());
        (self.shown.replace(now) != Some(now)).then_some(now)
    }
}

/// Whether `media` has a main-video line and the call has the link to switch it with.
pub fn available(media: &MediaSession, links: &Links, meeting: bool) -> bool {
    media.video_sender(ROLE).is_some()
        && if meeting {
            share::update_link(links).is_some()
        } else {
            links.get("mediaRenegotiation").is_some()
        }
}

/// Tells a meeting's media server that our camera sends (or stopped). The line stays `recvonly`
/// when off, so the meeting's video can still come in on it (UNVERIFIED).
pub async fn tell_meeting(
    ngc: &NgcClient,
    s: &CallSession,
    links: &Links,
    media: &MediaSession,
    camera: &mut Camera,
    on: bool,
) -> anyhow::Result<()> {
    let mid = media
        .video_mid(ROLE)
        .await
        .ok_or_else(|| anyhow::anyhow!("the video line has no mid"))?;
    let url =
        share::update_link(links).ok_or_else(|| anyhow::anyhow!("no updateMediaDescriptions"))?;
    let mut with = links.clone();
    with.0.insert("updateMediaDescriptions".into(), url);
    let tag = negotiation_tag(s, ROLE, on);
    let lines = [MediaDescription::line(mid, ROLE, on)];
    ngc.update_media_descriptions(s, &with, &lines, &tag, camera.next_request_id())
        .await?;
    Ok(())
}

/// The direction of our main-video line in a 1:1 re-offer: `sendrecv` while the camera is on,
/// `recvonly` while it is off (the other side's camera still comes in; UNVERIFIED).
pub fn direct_direction(on: bool) -> &'static str {
    if on { "sendrecv" } else { "recvonly" }
}

/// Re-offers a 1:1 call's media with the camera on or off, keeping the sharing line as it is
/// (`sharing`). Returns the answer when it came inline; otherwise it arrives on
/// `call/mediaAnswer/`.
pub async fn reoffer_direct(
    ngc: &NgcClient,
    s: &CallSession,
    links: &Links,
    media: &MediaSession,
    on: bool,
    sharing: bool,
) -> anyhow::Result<Option<SdpBlob>> {
    let offer = media.create_offer().await?;
    let mut offer = calling::media::sdp::with_role_direction(&offer, ROLE, direct_direction(on));
    if media.video_sender(VideoRole::ApplicationSharing).is_some() {
        offer = calling::media::sdp::with_role_direction(
            &offer,
            VideoRole::ApplicationSharing,
            if sharing { "sendonly" } else { "inactive" },
        );
    }
    let mut sending = Vec::new();
    if on {
        sending.push(ROLE);
    }
    if sharing {
        sending.push(VideoRole::ApplicationSharing);
    }
    let answer = ngc
        .renegotiate_video(
            s,
            links,
            &video_modalities(&sending),
            &SdpBlob::ngc(offer),
            &negotiation_tag(s, ROLE, on),
        )
        .await?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(path: &str, bus: &str) -> devices::Camera {
        devices::Camera {
            path: path.into(),
            name: "Webcam".into(),
            driver: "uvcvideo".into(),
            bus: bus.into(),
            formats: vec![],
        }
    }

    #[test]
    fn the_camera_is_opt_in_until_verified_live() {
        assert!(!env_on(None));
        assert!(!env_on(Some("0".as_ref())));
        assert!(!env_on(Some("".as_ref())));
        assert!(env_on(Some("1".as_ref())));
    }

    #[test]
    fn the_v4l2_fallback_uses_the_camera_chosen_in_settings() {
        let list = [cam("/dev/video0", "usb-1"), cam("/dev/video2", "usb-2")];
        assert_eq!(
            device_for(&list, Some("usb-2")).as_deref(),
            Some("/dev/video2")
        );
        assert_eq!(
            device_for(&list, Some("gone")).as_deref(),
            Some("/dev/video0")
        );
        assert_eq!(device_for(&list, None).as_deref(), Some("/dev/video0"));
        assert_eq!(device_for(&[], Some("usb-2")), None);
    }

    #[test]
    fn a_direct_call_keeps_receiving_video_with_the_camera_off() {
        assert_eq!(direct_direction(true), "sendrecv");
        assert_eq!(direct_direction(false), "recvonly");
    }

    #[test]
    fn the_ui_hears_only_changes() {
        let mut c = Camera::default();
        assert_eq!(c.changed(false), Some((false, false)));
        assert_eq!(c.changed(false), None);
        assert_eq!(c.changed(true), Some((true, false)));
    }

    #[test]
    fn accept_with_video_is_read_once() {
        // Without KELMA_CAMERA the flag never sets.
        if wanted() {
            return;
        }
        accept_with_video(true);
        assert!(!take_accept_with_video());
    }
}
