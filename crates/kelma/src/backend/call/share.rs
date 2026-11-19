//! Screen sharing in a live call: the desktop's ScreenCast picker, the encoder task feeding the
//! call's sharing line, and telling the service (see `docs/protocol-calls.md`, Screen sharing).
//!
//! The sharing line is negotiated only when `KELMA_SCREEN_SHARE=1` (until a live run confirms the
//! media servers accept it, the verified audio-only offer stays the default). Then:
//! - a meeting flips the line with `UpdateMediaDescriptions` (no renegotiation);
//! - a 1:1 call sends a re-offer with `ScreenSharer` on the other side's `mediaRenegotiation`.
//!
//! The camera will reuse this path with `VideoRole::MainVideo` and a camera frame source.

use calling::media::{MediaOptions, MediaSession};
use calling::signaling::video::{
    MediaDescription, negotiation_tag, parse_sender_controls, video_modalities,
};
use calling::signaling::{CallSession, NgcClient};
use calling::video::h264::{EncoderSettings, H264Encoder};
use calling::video::screencast::{CaptureSettings, ScreenCapture, ShareKind, pick};
use calling::video::{VideoRole, pump};
use calling::{Links, SdpBlob};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const ROLE: VideoRole = VideoRole::ApplicationSharing;

/// Whether calls negotiate a sharing line (`KELMA_SCREEN_SHARE` set to anything but `0`).
pub fn wanted() -> bool {
    wanted_by(std::env::var_os("KELMA_SCREEN_SHARE").as_deref())
}

fn wanted_by(v: Option<&std::ffi::OsStr>) -> bool {
    v.is_some_and(|v| !v.is_empty() && v != "0")
}

/// The media options of a call's session: audio, plus the sharing line when [`wanted`].
pub fn media_options() -> MediaOptions {
    MediaOptions {
        video: if wanted() { vec![ROLE] } else { vec![] },
        ..MediaOptions::default()
    }
}

/// What the share tasks report to the call loop.
pub enum ShareMsg {
    /// The picker closed: a capture, `None` when cancelled, or why it failed.
    Picked(Result<Option<Box<ScreenCapture>>, String>),
    /// The capture ended on its own (the user stopped it from the desktop's indicator).
    Ended,
}

/// One call's share state.
#[derive(Default)]
pub struct Share {
    picking: bool,
    capture: Option<ScreenCapture>,
    encoder: Option<JoinHandle<()>>,
    keyframes: Option<mpsc::UnboundedSender<()>>,
    /// `requestId` of the last `UpdateMediaDescriptions`.
    request_id: u64,
    /// What the UI was last told: (available, sharing).
    shown: Option<(bool, bool)>,
}

impl Share {
    pub fn sharing(&self) -> bool {
        self.capture.is_some()
    }

    /// Opens the desktop's picker without blocking the call; the result comes as
    /// [`ShareMsg::Picked`].
    pub fn pick(&mut self, tx: &mpsc::UnboundedSender<ShareMsg>) {
        if self.picking || self.sharing() {
            return;
        }
        self.picking = true;
        let tx = tx.clone();
        tokio::spawn(async move {
            let r = pick(ShareKind::Any, CaptureSettings::default())
                .await
                .map(|c| c.map(Box::new))
                .map_err(|e| e.to_string());
            let _ = tx.send(ShareMsg::Picked(r));
        });
    }

    /// Starts sending `capture` on `media`'s sharing line. The service is told by the caller.
    pub fn start(
        &mut self,
        mut capture: ScreenCapture,
        media: &MediaSession,
        tx: &mpsc::UnboundedSender<ShareMsg>,
    ) -> Result<(), String> {
        self.picking = false;
        let sender = media
            .video_sender(ROLE)
            .ok_or("this call has no screen-sharing line")?;
        let encoder = H264Encoder::new(EncoderSettings::screen()).map_err(|e| e.to_string())?;
        let (keys_tx, keys_rx) = mpsc::unbounded_channel();
        let task = pump(capture.take_frames(), encoder, sender, keys_rx);
        let tx = tx.clone();
        // The pump ends when the capture does; the watcher reports it unless we stopped it.
        let watcher = tokio::spawn(async move {
            let _ = task.await;
            let _ = tx.send(ShareMsg::Ended);
        });
        self.capture = Some(capture);
        self.encoder = Some(watcher);
        self.keyframes = Some(keys_tx);
        Ok(())
    }

    /// Stops the capture and the encoder. True when something was being shared.
    pub async fn stop(&mut self) -> bool {
        self.picking = false;
        self.keyframes = None;
        if let Some(e) = self.encoder.take() {
            e.abort();
        }
        match self.capture.take() {
            Some(c) => {
                c.stop().await;
                true
            }
            None => false,
        }
    }

    pub fn picked_nothing(&mut self) {
        self.picking = false;
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
        let now = (available, self.sharing());
        (self.shown.replace(now) != Some(now)).then_some(now)
    }
}

/// Whether `media` has a sharing line and the call has the link to switch it with.
pub fn available(media: &MediaSession, links: &Links, meeting: bool) -> bool {
    media.video_sender(ROLE).is_some()
        && if meeting {
            update_link(links).is_some()
        } else {
            links.get("mediaRenegotiation").is_some()
        }
}

/// The call leg's `updateMediaDescriptions` link, or `{callLeg}/updateMediaDescriptions` (equal
/// in the web client's capture, mautrix-teams openapi).
fn update_link(links: &Links) -> Option<String> {
    links
        .get("updateMediaDescriptions")
        .map(str::to_owned)
        .or_else(|| {
            links
                .get("callLeg")
                .map(|leg| format!("{}/updateMediaDescriptions", leg.trim_end_matches('/')))
        })
}

/// Tells a meeting's media server that our sharing line sends (or stopped).
pub async fn tell_meeting(
    ngc: &NgcClient,
    s: &CallSession,
    links: &Links,
    media: &MediaSession,
    share: &mut Share,
    sending: bool,
) -> anyhow::Result<()> {
    let mid = media
        .video_mid(ROLE)
        .await
        .ok_or_else(|| anyhow::anyhow!("the sharing line has no mid"))?;
    let url = update_link(links).ok_or_else(|| anyhow::anyhow!("no updateMediaDescriptions"))?;
    let mut with = links.clone();
    with.0.insert("updateMediaDescriptions".into(), url);
    let tag = negotiation_tag(s, ROLE, sending);
    let lines = [MediaDescription::line(mid, ROLE, sending)];
    ngc.update_media_descriptions(s, &with, &lines, &tag, share.next_request_id())
        .await?;
    Ok(())
}

/// Re-offers a 1:1 call's media with the sharing line `sendonly` (or `inactive`). Returns the
/// answer when it came inline; otherwise it arrives on `call/mediaAnswer/`.
pub async fn reoffer_direct(
    ngc: &NgcClient,
    s: &CallSession,
    links: &Links,
    media: &MediaSession,
    sending: bool,
) -> anyhow::Result<Option<SdpBlob>> {
    let offer = media.create_offer().await?;
    let offer = calling::media::sdp::with_role_direction(
        &offer,
        ROLE,
        if sending { "sendonly" } else { "inactive" },
    );
    let sending_roles: &[VideoRole] = if sending { &[ROLE] } else { &[] };
    let answer = ngc
        .renegotiate_video(
            s,
            links,
            &video_modalities(sending_roles),
            &SdpBlob::ngc(offer),
            &negotiation_tag(s, ROLE, sending),
        )
        .await?;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_is_opt_in_until_verified_live() {
        assert!(!wanted_by(None));
        assert!(!wanted_by(Some("0".as_ref())));
        assert!(!wanted_by(Some("".as_ref())));
        assert!(wanted_by(Some("1".as_ref())));
    }

    #[test]
    fn the_update_link_falls_back_to_the_call_leg() {
        let mut links = Links::default();
        assert_eq!(update_link(&links), None);
        links
            .0
            .insert("callLeg".into(), "https://cc.example.invalid/leg/7/".into());
        assert_eq!(
            update_link(&links).as_deref(),
            Some("https://cc.example.invalid/leg/7/updateMediaDescriptions")
        );
        links.0.insert(
            "updateMediaDescriptions".into(),
            "https://cc.example.invalid/u".into(),
        );
        assert_eq!(
            update_link(&links).as_deref(),
            Some("https://cc.example.invalid/u")
        );
    }

    #[test]
    fn the_ui_hears_only_changes() {
        let mut s = Share::default();
        assert_eq!(s.changed(false), Some((false, false)));
        assert_eq!(s.changed(false), None);
        assert_eq!(s.changed(true), Some((true, false)));
    }
}
