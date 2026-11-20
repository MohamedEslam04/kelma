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
use calling::signaling::content_sharing::{
    ADD_MODALITY_FAILURE, ADD_MODALITY_SUCCESS, SESSION_END, SESSION_UPDATE, SharingSession, new_id,
};
use calling::signaling::video::{
    JOIN_REQUEST_ID, meeting_descriptions, negotiation_tag, parse_sender_controls, video_modalities,
};
use calling::signaling::{CallSession, NgcClient};
use calling::video::h264::{EncoderSettings, H264Encoder};
use calling::video::remote_input::{InputInjector, pick_with_control};
use calling::video::screencast::{CaptureSettings, ScreenCapture, ShareKind, pick};
use calling::video::{VideoRole, pump};
use calling::{Links, SdpBlob};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub const ROLE: VideoRole = VideoRole::ApplicationSharing;

/// Whether calls negotiate a sharing line (`KELMA_SCREEN_SHARE` set to anything but `0`).
pub fn wanted() -> bool {
    wanted_by(std::env::var_os("KELMA_SCREEN_SHARE").as_deref())
}

fn wanted_by(v: Option<&std::ffi::OsStr>) -> bool {
    v.is_some_and(|v| !v.is_empty() && v != "0")
}

/// The media options of a call's session: audio, plus the camera's main-video line when
/// [`super::camera::wanted`] and the sharing line when [`wanted`].
pub fn media_options() -> MediaOptions {
    let mut video = Vec::new();
    if super::camera::wanted() {
        video.push(super::camera::ROLE);
    }
    if wanted() {
        video.push(ROLE);
    }
    MediaOptions {
        video,
        ..MediaOptions::default()
    }
}

/// What the picker hands back: the capture, and the remote-control injector when the share was
/// started in a combined RemoteDesktop session ([`super::remote_control::wanted`]).
pub struct Picked {
    pub capture: ScreenCapture,
    pub injector: Option<InputInjector>,
}

/// What the share tasks report to the call loop.
pub enum ShareMsg {
    /// The picker closed: a capture, `None` when cancelled, or why it failed.
    Picked(Result<Option<Box<Picked>>, String>),
    /// The capture ended on its own (the user stopped it from the desktop's indicator).
    Ended,
    /// The meeting did not grant the content-sharing session in time (correlation id).
    GrantTimeout(String),
}

/// How long a meeting may take to grant the content-sharing session before the share is sent
/// anyway (UNVERIFIED: the web client waits for the grant).
const GRANT_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// What a content-sharing callback changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionNews {
    /// The session we asked for was granted: the share can go out.
    Granted,
    /// The meeting refused it.
    Refused,
    /// Our session ended (someone else presents now, or the meeting ended it).
    Ended,
    Nothing,
}

/// One call's share state.
#[derive(Default)]
pub struct Share {
    picking: bool,
    capture: Option<ScreenCapture>,
    encoder: Option<JoinHandle<()>>,
    keyframes: Option<mpsc::UnboundedSender<()>>,
    /// `requestId` of the last `UpdateMediaDescriptions` (the camera's included).
    request_id: u64,
    /// What the UI was last told: (available, sharing).
    shown: Option<(bool, bool)>,
    /// The meeting's content-sharing session we present in (never merged into the call's links).
    pub session: Option<SharingSession>,
    /// The correlation id of the session we asked for, until it is granted.
    asking: Option<String>,
    /// Drives our screen for the person we gave control to (combined portal session).
    pub injector: Option<InputInjector>,
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
        let with_control = super::remote_control::wanted();
        tokio::spawn(async move {
            let (kind, settings) = (ShareKind::Any, CaptureSettings::default());
            let r = if with_control {
                // One consent for the screen and for being controlled (RemoteDesktop portal).
                pick_with_control(kind, settings).await.map(|p| {
                    p.map(|(capture, injector)| Picked {
                        capture,
                        injector: Some(injector),
                    })
                })
            } else {
                pick(kind, settings).await.map(|c| {
                    c.map(|capture| Picked {
                        capture,
                        injector: None,
                    })
                })
            };
            let r = r.map(|p| p.map(Box::new)).map_err(|e| e.to_string());
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

    /// Stops the capture and the encoder. True when something was being shared. The
    /// content-sharing session is given back by the caller ([`leave_session`]).
    pub async fn stop(&mut self) -> bool {
        self.picking = false;
        self.keyframes = None;
        self.injector = None;
        self.asking = None;
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

    /// A conversation callback about the content-sharing session (`event` is its name).
    pub fn session_frame(&mut self, event: &str, body: &serde_json::Value) -> SessionNews {
        match event {
            ADD_MODALITY_SUCCESS | SESSION_UPDATE => {
                let corr = self
                    .asking
                    .clone()
                    .or_else(|| self.session.as_ref().map(|s| s.correlation_id.clone()));
                let Some(corr) = corr else {
                    return SessionNews::Nothing;
                };
                match SharingSession::read(&corr, body) {
                    Some(granted) => {
                        self.session = Some(granted);
                        if self.asking.take().is_some() {
                            SessionNews::Granted
                        } else {
                            SessionNews::Nothing
                        }
                    }
                    None => SessionNews::Nothing,
                }
            }
            ADD_MODALITY_FAILURE if self.asking.take().is_some() => SessionNews::Refused,
            SESSION_END if self.session.take().is_some() => SessionNews::Ended,
            _ => SessionNews::Nothing,
        }
    }

    /// The next `requestId` of the call's `UpdateMediaDescriptions` (the camera's too: one count
    /// per call, on from the one the join declared its lines with).
    pub fn next_request_id(&mut self) -> u64 {
        self.request_id = self.request_id.max(JOIN_REQUEST_ID) + 1;
        self.request_id
    }

    /// The state to show, when it changed since last shown.
    pub fn changed(&mut self, available: bool) -> Option<(bool, bool)> {
        let now = (available, self.sharing());
        (self.shown.replace(now) != Some(now)).then_some(now)
    }
}

/// Whether `media` has a sharing line and the call has the link to switch it with. In a meeting
/// the line must also be live: the media servers answer a line they will not carry with port 0.
pub async fn available(media: &MediaSession, links: &Links, meeting: bool) -> bool {
    line_available(media, links, meeting, ROLE).await
}

/// [`available`] for `role`'s line (the camera's is [`super::camera::available`]).
pub(super) async fn line_available(
    media: &MediaSession,
    links: &Links,
    meeting: bool,
    role: VideoRole,
) -> bool {
    if media.video_sender(role).is_none() {
        return false;
    }
    if meeting {
        update_link(links).is_some() && media.live_video_mid(role).await.is_some()
    } else {
        links.get("mediaRenegotiation").is_some()
    }
}

/// What the call says when a meeting's media server refused `role`'s line.
pub fn unavailable_text(role: VideoRole) -> &'static str {
    match role {
        VideoRole::MainVideo => "Video isn't available in this meeting.",
        VideoRole::ApplicationSharing => "Sharing isn't available in this meeting.",
    }
}

/// Which of our lines send in a meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sending {
    pub camera: bool,
    pub share: bool,
}

impl Sending {
    fn of(self, role: VideoRole) -> bool {
        match role {
            VideoRole::MainVideo => self.camera,
            VideoRole::ApplicationSharing => self.share,
        }
    }
}

/// The call leg's `updateMediaDescriptions` link, or `{callLeg}/updateMediaDescriptions` (equal
/// in the web client's capture, mautrix-teams openapi).
pub(super) fn update_link(links: &Links) -> Option<String> {
    links
        .get("updateMediaDescriptions")
        .map(str::to_owned)
        .or_else(|| {
            links
                .get("callLeg")
                .map(|leg| format!("{}/updateMediaDescriptions", leg.trim_end_matches('/')))
        })
}

/// Asks a meeting for its content-sharing session before our share goes out (see
/// `calling::signaling::content_sharing`, UNVERIFIED). Returns true when the share can go out
/// now: the grant came in the answer, or the call names no `addModality` link (the share then
/// goes out without a session, as before). Otherwise the grant comes on a callback
/// ([`Share::session_frame`]) or [`ShareMsg::GrantTimeout`] fires.
pub async fn start_session(
    ngc: &NgcClient,
    s: &CallSession,
    links: &Links,
    share: &mut Share,
    tx: &mpsc::UnboundedSender<ShareMsg>,
) -> anyhow::Result<bool> {
    let Some(url) = links.get("addModality") else {
        tracing::info!("no addModality link: sharing without a content-sharing session");
        return Ok(true);
    };
    let correlation = new_id();
    let answer = ngc
        .start_content_sharing(s, url, &correlation, &new_id())
        .await?;
    if let Some(granted) = SharingSession::read(&correlation, &answer.body) {
        share.session = Some(granted);
        return Ok(true);
    }
    share.asking = Some(correlation.clone());
    let tx = tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(GRANT_WAIT).await;
        let _ = tx.send(ShareMsg::GrantTimeout(correlation));
    });
    Ok(false)
}

/// The grant did not come: stop waiting for it (true when we still were).
pub fn grant_timed_out(share: &mut Share, correlation: &str) -> bool {
    if share.asking.as_deref() == Some(correlation) {
        share.asking = None;
        true
    } else {
        false
    }
}

/// Gives the content-sharing session back on its own `leave` link, if we hold one.
pub async fn leave_session(ngc: &NgcClient, s: &CallSession, share: &mut Share) {
    share.asking = None;
    if let Some(session) = share.session.take()
        && let Err(e) = ngc.leave_content_sharing(s, &session).await
    {
        tracing::warn!("content-sharing session not given back: {e:#}");
    }
}

/// Tells a meeting's media server, for a change of `role`'s line, what every live video line of
/// ours does now: the whole list, receive-only but for those `sending`, as the web client and
/// mautrix-teams (`SetVideo`) post it. Posts nothing when `role`'s line is not live: the server
/// refuses an update naming a line it answered with port 0 (400, 1047 "Invalid
/// MediaDescriptions", live 2026-11-20) and then ends the call (410).
pub async fn tell_meeting(
    ngc: &NgcClient,
    s: &CallSession,
    links: &Links,
    media: &MediaSession,
    role: VideoRole,
    sending: Sending,
    request_id: u64,
) -> anyhow::Result<()> {
    if media.live_video_mid(role).await.is_none() {
        anyhow::bail!("{}", unavailable_text(role));
    }
    let url = update_link(links).ok_or_else(|| anyhow::anyhow!("no updateMediaDescriptions"))?;
    let mut with = links.clone();
    with.0.insert("updateMediaDescriptions".into(), url);
    let mut sends = Vec::new();
    for r in [VideoRole::MainVideo, VideoRole::ApplicationSharing] {
        if sending.of(r)
            && let Some(mid) = media.live_video_mid(r).await
        {
            sends.push((mid, r));
        }
    }
    let lines = meeting_descriptions(&media.live_video_mids(), &sends);
    let tag = negotiation_tag(s, role, sending.of(role));
    ngc.update_media_descriptions(s, &with, &lines, &tag, request_id)
        .await?;
    Ok(())
}

/// A 1:1 re-offer (`offer`, as the media session made it) for the share: the sharing line
/// `sendonly` (or `inactive`), and the camera line, when there is one (`camera_on` is `Some`),
/// as the camera is ([`super::camera::direct_direction`]): webrtc-rs leaves it `sendonly` from
/// the answer to a caller who offered it `recvonly`, which said our camera sends while it was off.
pub fn direct_offer(offer: &str, sending: bool, camera_on: Option<bool>) -> String {
    let mut offer = calling::media::sdp::with_role_direction(
        offer,
        ROLE,
        if sending { "sendonly" } else { "inactive" },
    );
    if let Some(on) = camera_on {
        offer = calling::media::sdp::with_role_direction(
            &offer,
            VideoRole::MainVideo,
            super::camera::direct_direction(on),
        );
    }
    offer
}

/// Re-offers a 1:1 call's media with the sharing line `sendonly` (or `inactive`) and the camera
/// line as the camera is (`camera_on`). Returns the answer when it came inline; otherwise it
/// arrives on `call/mediaAnswer/`.
pub async fn reoffer_direct(
    ngc: &NgcClient,
    s: &CallSession,
    links: &Links,
    media: &MediaSession,
    sending: bool,
    camera_on: bool,
) -> anyhow::Result<Option<SdpBlob>> {
    let offer = media.create_offer().await?;
    let camera = media
        .video_sender(VideoRole::MainVideo)
        .is_some()
        .then_some(camera_on);
    let offer = direct_offer(&offer, sending, camera);
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
    fn a_session_is_granted_by_its_callback_and_ended_by_its_end() {
        let mut s = Share {
            asking: Some("corr".into()),
            ..Default::default()
        };
        assert!(s.asking.is_some());
        let grant = serde_json::json!({"links": {"leave": "https://x/cs-leave"}});
        assert_eq!(
            s.session_frame("conversationUpdate", &grant),
            SessionNews::Nothing
        );
        assert_eq!(
            s.session_frame("contentSharingUpdate", &grant),
            SessionNews::Granted
        );
        assert!(!s.asking.is_some());
        assert_eq!(
            s.session.as_ref().and_then(|x| x.leave()),
            Some("https://x/cs-leave")
        );
        assert_eq!(s.session.as_ref().unwrap().correlation_id, "corr");
        // A later update refreshes the links; nothing to send again.
        assert_eq!(
            s.session_frame("contentSharingUpdate", &grant),
            SessionNews::Nothing
        );
        assert_eq!(
            s.session_frame("contentSharingEnd", &grant),
            SessionNews::Ended
        );
        assert!(s.session.is_none());
        assert_eq!(
            s.session_frame("contentSharingEnd", &grant),
            SessionNews::Nothing
        );
    }

    #[test]
    fn a_refused_or_late_grant_stops_the_wait_once() {
        let mut s = Share {
            asking: Some("c1".into()),
            ..Default::default()
        };
        assert!(!grant_timed_out(&mut s, "c0"), "an older ask");
        assert!(grant_timed_out(&mut s, "c1"));
        assert!(!grant_timed_out(&mut s, "c1"));
        s.asking = Some("c2".into());
        let nothing = serde_json::json!({});
        assert_eq!(
            s.session_frame("addModalityFailure", &nothing),
            SessionNews::Refused
        );
        assert_eq!(
            s.session_frame("addModalityFailure", &nothing),
            SessionNews::Nothing
        );
        // Without an ask, a stray grant is not ours.
        assert_eq!(
            s.session_frame(
                "addModalitySuccess",
                &serde_json::json!({"links": {"leave": "x"}})
            ),
            SessionNews::Nothing
        );
        assert!(s.session.is_none());
    }

    /// The live 1:1 share re-offer (2026-11-20) said `sendonly` on the camera line with the
    /// camera off: the sharing line alone sends.
    #[test]
    fn a_direct_share_offer_sends_on_the_sharing_line_only() {
        let offer = "v=0\r\n\
            m=video 9 RTP/SAVP 107\r\na=mid:1\r\na=sendonly\r\na=label:main-video\r\n\
            m=video 9 RTP/SAVP 107\r\na=mid:2\r\na=inactive\r\n\
            a=label:applicationsharing-video\r\n";
        let directions = |sdp: &str| -> Vec<String> {
            sdp.lines()
                .filter(|l| ["a=sendrecv", "a=sendonly", "a=recvonly", "a=inactive"].contains(l))
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(
            directions(&direct_offer(offer, true, Some(false))),
            ["a=recvonly", "a=sendonly"]
        );
        assert_eq!(
            directions(&direct_offer(offer, true, Some(true))),
            ["a=sendrecv", "a=sendonly"]
        );
        assert_eq!(
            directions(&direct_offer(offer, false, None)),
            ["a=sendonly", "a=inactive"]
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
