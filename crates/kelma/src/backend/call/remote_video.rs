//! Other people's video in a live call: which of our lines each picture came on, who a meeting's
//! gallery slots show, and who presents (see `docs/protocol-calls.md`, Remote video).
//!
//! Every remote video packet carries the mid of its line. One decoder per line:
//! - the camera line (`main-video`): the other person in a 1:1 call ([`VideoSlot::Remote`]);
//! - the sharing line (`applicationsharing-video`): a shared screen ([`VideoSlot::Content`]);
//! - a meeting's receive-only gallery slots ([`VideoSlot::Gallery`]): whoever the slot is
//!   subscribed to with `applyChannelParameters` (the participant's camera MSI from the roster on
//!   the slot's `x-source-streamid` from the media server's answer).
//!
//! The gallery slots are offered only with `KELMA_CAMERA=1` (as the camera line), the sharing
//! line only with `KELMA_SCREEN_SHARE=1`. Everything here is UNVERIFIED live.

use super::{EventSink, VideoSlot};
use calling::Links;
use calling::media::MediaSession;
use calling::media::sdp::source_stream_ids;
use calling::signaling::roster::Roster;
use calling::signaling::video::{SCREEN_LIMITS, SourceRequest, TILE_LIMITS, plan_receive};
use calling::signaling::{CallSession, NgcClient};
use calling::video::receive::{VideoPacket, spawn_decoder};
use calling::video::{RgbFrame, VideoRole};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// The gallery's receive-only lines a meeting join offers (as the web client and rusty-teams).
pub const RECEIVE_SLOTS: usize = 9;
/// The other side's camera in a 1:1 call, in their tile: no wider than this.
const REMOTE_WIDTH: usize = 960;
/// A gallery tile.
const TILE_WIDTH: usize = 640;
/// A shared screen, on the whole stage.
const CONTENT_WIDTH: usize = 1920;
/// Packets waiting for one line's decoder.
const LINE_QUEUE: usize = 512;
/// A line that sends no picture for this long is cleared (a share that stopped, a camera that
/// went off without a renegotiation).
const IDLE: Duration = Duration::from_secs(10);

/// The gallery slots a meeting join offers: [`RECEIVE_SLOTS`] with `KELMA_CAMERA=1`, else none.
pub fn wanted_slots() -> usize {
    if super::camera::wanted() {
        RECEIVE_SLOTS
    } else {
        0
    }
}

/// Which picture each of our lines (by mid) draws.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Routes {
    share: Option<String>,
    slots: Vec<String>,
}

impl Routes {
    /// The sharing line is the stage's content, a gallery line its slot; anything else (the
    /// camera line, a line not known yet) is the other side's video, as before lines were told
    /// apart.
    fn slot_for(&self, mid: Option<&str>) -> VideoSlot {
        let Some(mid) = mid else {
            return VideoSlot::Remote;
        };
        if self.share.as_deref() == Some(mid) {
            return VideoSlot::Content;
        }
        match self.slots.iter().position(|s| s == mid) {
            Some(i) => VideoSlot::Gallery(i),
            None => VideoSlot::Remote,
        }
    }
}

fn width_for(slot: VideoSlot) -> usize {
    match slot {
        VideoSlot::Content => CONTENT_WIDTH,
        VideoSlot::Gallery(_) => TILE_WIDTH,
        VideoSlot::Remote | VideoSlot::SelfView => REMOTE_WIDTH,
    }
}

/// The call leg's `applyChannelParameters` link, or `{callLeg}/applyChannelParameters` (as the
/// `updateMediaDescriptions` fallback; UNVERIFIED).
fn apply_link(links: &Links) -> Option<String> {
    links
        .get("applyChannelParameters")
        .map(str::to_owned)
        .or_else(|| {
            links
                .get("callLeg")
                .map(|leg| format!("{}/applyChannelParameters", leg.trim_end_matches('/')))
        })
}

/// One call's remote video.
#[derive(Default)]
pub struct RemoteVideo {
    routes: Arc<Mutex<Routes>>,
    /// The gallery slots: (mid, `x-source-streamid`).
    table: Vec<(String, u32)>,
    /// The sharing line, when the answer gave it a stream id.
    share_line: Option<(String, u32)>,
    /// Who each slot is subscribed to: (MRI, camera MSI).
    held: Vec<Option<(String, u32)>>,
    /// The share subscribed to: (MRI, share MSI).
    share_held: Option<(String, u32)>,
    /// The `sequenceNumber` of the last subscription.
    sequence: u64,
    /// Sends the subscriptions in order.
    requests: Option<mpsc::UnboundedSender<(String, SourceRequest, u64)>>,
    router: Option<JoinHandle<()>>,
    /// The presenter's name the UI was last told.
    presenter: Option<String>,
}

impl RemoteVideo {
    /// Draws the video of a session's [`calling::media::MediaEvents::video`]; replaces what a
    /// previous session drew.
    pub fn show(&mut self, mut packets: mpsc::Receiver<VideoPacket>, sink: &EventSink) {
        if let Some(r) = self.router.take() {
            r.abort();
        }
        let routes = Arc::clone(&self.routes);
        let sink = sink.clone();
        self.router = Some(tokio::spawn(async move {
            let mut lines: HashMap<Option<Arc<str>>, mpsc::Sender<VideoPacket>> = HashMap::new();
            while let Some(mut p) = packets.recv().await {
                let tx = lines.entry(p.mid.clone()).or_insert_with(|| {
                    let slot = lock(&routes).slot_for(p.mid.as_deref());
                    let (tx, rx) = mpsc::channel(LINE_QUEUE);
                    let frames = spawn_decoder(rx, width_for(slot));
                    tokio::spawn(forward_frames(
                        frames,
                        p.mid.clone(),
                        Arc::clone(&routes),
                        sink.clone(),
                    ));
                    tx
                });
                // One line is one picture, whichever track (SSRC) webrtc-rs reports it on: a
                // slot that changes person may come on a new one.
                if let Some(mid) = &p.mid {
                    p.track = Arc::clone(mid);
                }
                let _ = tx.try_send(p);
            }
        }));
    }

    /// Stops drawing and clears every remote picture.
    pub fn stop(&mut self, sink: &EventSink) {
        if let Some(r) = self.router.take() {
            r.abort();
        }
        sink.video(VideoSlot::Remote, None);
        sink.video(VideoSlot::Content, None);
        for i in 0..self.table.len() {
            sink.video(VideoSlot::Gallery(i), None);
        }
    }

    /// Reads the lines of a negotiated session: its gallery slots' and sharing line's mids, and
    /// their `x-source-streamid`s in the media server's SDP (`sdp`, an answer or its offer).
    /// Every subscription starts over.
    pub async fn negotiated(&mut self, media: &MediaSession, sdp: &str) {
        let ids = source_stream_ids(sdp);
        let share = media.video_mid(VideoRole::ApplicationSharing).await;
        self.table = media
            .slot_mids()
            .await
            .into_iter()
            .filter_map(|mid| {
                let id = *ids.get(&mid)?;
                Some((mid, id))
            })
            .collect();
        self.share_line = share
            .as_ref()
            .and_then(|mid| Some((mid.clone(), *ids.get(mid)?)));
        *lock(&self.routes) = Routes {
            share,
            slots: self.table.iter().map(|(mid, _)| mid.clone()).collect(),
        };
        self.held = vec![None; self.table.len()];
        self.share_held = None;
        tracing::info!(
            slots = self.table.len(),
            share = self.share_line.is_some(),
            "remote video lines"
        );
    }

    /// The gallery slot showing `mri`'s camera.
    pub fn slot_of(&self, mri: &str) -> Option<usize> {
        self.held
            .iter()
            .position(|h| h.as_ref().is_some_and(|(m, _)| m == mri))
    }

    /// The subscriptions the roster now calls for: the others' cameras into the slots (the
    /// latest speakers first, by `spoke`), and the share someone else presents onto the sharing
    /// line. Slots whose person changed are cleared through `cleared`.
    fn plan(
        &mut self,
        roster: &Roster,
        me: &str,
        spoke: impl Fn(&str) -> u64,
        mut cleared: impl FnMut(usize),
    ) -> Vec<SourceRequest> {
        let mut wanted = roster.video_candidates(me);
        // Stable: people who never spoke keep the roster's order.
        wanted.sort_by_key(|(mri, _)| std::cmp::Reverse(spoke(mri)));
        let next = plan_receive(&self.held, &wanted);
        let mut out = Vec::new();
        for (i, ((mid, msid), (before, after))) in self
            .table
            .iter()
            .zip(self.held.iter().zip(&next))
            .enumerate()
        {
            if before == after {
                continue;
            }
            if before.as_ref().map(|b| &b.0) != after.as_ref().map(|a| &a.0) {
                cleared(i);
            }
            out.push(SourceRequest {
                mid: mid.clone(),
                stream_msid: *msid,
                source: after.as_ref().map(|(_, source)| *source),
                limits: TILE_LIMITS,
            });
        }
        self.held = next;
        if let Some((mid, msid)) = &self.share_line {
            let sharer = roster.screen_sharer(me);
            if sharer != self.share_held {
                out.push(SourceRequest {
                    mid: mid.clone(),
                    stream_msid: *msid,
                    source: sharer.as_ref().map(|(_, source)| *source),
                    limits: SCREEN_LIMITS,
                });
                self.share_held = sharer;
            }
        }
        out
    }

    /// Follows a roster change in a meeting: sends the subscriptions it calls for.
    pub fn reselect(
        &mut self,
        ngc: &NgcClient,
        s: &CallSession,
        links: &Links,
        roster: &Roster,
        spoke: impl Fn(&str) -> u64,
        sink: &EventSink,
    ) {
        let me = s.me.mri.clone();
        let requests = self.plan(roster, &me, spoke, |i| {
            sink.video(VideoSlot::Gallery(i), None);
        });
        if requests.is_empty() {
            return;
        }
        let Some(url) = apply_link(links) else {
            tracing::warn!("no applyChannelParameters link: remote video not subscribed");
            return;
        };
        let queue = self.requests.get_or_insert_with(|| {
            let (tx, mut rx) = mpsc::unbounded_channel::<(String, SourceRequest, u64)>();
            let (ngc, s) = (ngc.clone(), s.clone());
            tokio::spawn(async move {
                while let Some((url, r, n)) = rx.recv().await {
                    match ngc.apply_channel_parameters(&s, &url, &r, n).await {
                        Ok(()) => {
                            tracing::info!(mid = %r.mid, source = ?r.source, "video subscribed")
                        }
                        Err(e) => tracing::warn!(%e, mid = %r.mid, "video subscription refused"),
                    }
                }
            });
            tx
        });
        for r in requests {
            self.sequence += 1;
            let _ = queue.send((url.clone(), r, self.sequence));
        }
    }

    /// The presenter's name when it changed since last told: someone else in the roster shares
    /// a screen (empty: nobody).
    pub fn presenter_changed(&mut self, roster: &Roster, me: &str) -> Option<String> {
        let name = roster
            .screen_sharer(me)
            .and_then(|(mri, _)| roster.get(&mri).map(|p| p.display_name.clone()))
            .map(|n| {
                if n.is_empty() {
                    "Someone".to_owned()
                } else {
                    n
                }
            })
            .unwrap_or_default();
        (self.presenter.as_ref() != Some(&name)).then(|| {
            self.presenter = Some(name.clone());
            name
        })
    }
}

fn lock(routes: &Mutex<Routes>) -> std::sync::MutexGuard<'_, Routes> {
    routes.lock().unwrap_or_else(|e| e.into_inner())
}

/// Draws one line's pictures where the line's route says, until its decoder ends; a line idle
/// for [`IDLE`] is cleared until it sends again.
async fn forward_frames(
    mut frames: mpsc::Receiver<RgbFrame>,
    mid: Option<Arc<str>>,
    routes: Arc<Mutex<Routes>>,
    sink: EventSink,
) {
    let mut drawn: Option<VideoSlot> = None;
    loop {
        match tokio::time::timeout(IDLE, frames.recv()).await {
            Ok(Some(f)) => {
                let slot = lock(&routes).slot_for(mid.as_deref());
                if let Some(old) = drawn.filter(|old| *old != slot) {
                    sink.video(old, None);
                }
                drawn = Some(slot);
                sink.video(slot, Some(f));
            }
            Ok(None) => break,
            Err(_) => {
                if let Some(old) = drawn.take() {
                    sink.video(old, None);
                }
            }
        }
    }
    if let Some(old) = drawn {
        sink.video(old, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn roster(people: serde_json::Value) -> Roster {
        let mut r = Roster::default();
        r.apply(&json!({"type": "Delta", "participants": people}));
        r
    }

    fn person(name: &str, camera: Option<u32>, screen: Option<u32>) -> serde_json::Value {
        let mut streams = vec![json!({"type": "audio", "sourceId": 1, "direction": "sendrecv"})];
        if let Some(c) = camera {
            streams.push(json!({"type": "video", "label": "main-video", "sourceId": c, "direction": "sendonly"}));
        }
        if let Some(s) = screen {
            streams.push(
                json!({"type": "applicationsharing-video", "sourceId": s, "direction": "sendrecv"}),
            );
        }
        json!({"version": 1, "state": "active", "details": {"displayName": name},
               "endpoints": {"ep": {"call": {"mediaStreams": streams}}}})
    }

    fn with_lines(slots: usize, share: bool) -> RemoteVideo {
        RemoteVideo {
            table: (0..slots)
                .map(|i| (format!("{}", i + 3), 4000 + i as u32))
                .collect(),
            held: vec![None; slots],
            share_line: share.then(|| ("2".to_owned(), 3000)),
            ..Default::default()
        }
    }

    #[test]
    fn lines_route_to_the_stage_a_slot_or_the_other_side() {
        let r = Routes {
            share: Some("2".into()),
            slots: vec!["3".into(), "4".into()],
        };
        assert_eq!(r.slot_for(Some("2")), VideoSlot::Content);
        assert_eq!(r.slot_for(Some("4")), VideoSlot::Gallery(1));
        assert_eq!(r.slot_for(Some("1")), VideoSlot::Remote);
        assert_eq!(r.slot_for(None), VideoSlot::Remote);
    }

    #[test]
    fn the_latest_speakers_get_the_slots_and_keep_them() {
        let mut v = with_lines(2, false);
        let r = roster(json!({
            "8:me": person("Me", Some(10), None),
            "8:a": person("Ada", Some(11), None),
            "8:b": person("Bo", Some(12), None),
            "8:c": person("Cy", Some(13), None),
            "8:d": person("Di", None, None),
        }));
        let spoke = |m: &str| if m == "8:c" { 5 } else { 0 };
        let mut cleared = Vec::new();
        let first = v.plan(&r, "8:me", spoke, |i| cleared.push(i));
        let asked: Vec<_> = first.iter().map(|q| (q.mid.as_str(), q.source)).collect();
        assert_eq!(asked, [("3", Some(13)), ("4", Some(11))]);
        assert_eq!(first[0].stream_msid, 4000);
        assert_eq!(v.slot_of("8:c"), Some(0));
        assert_eq!(v.slot_of("8:me"), None);
        assert!(
            v.plan(&r, "8:me", spoke, |_| {}).is_empty(),
            "nothing changed"
        );
        assert_eq!(cleared, [0, 1]);
    }

    #[test]
    fn a_camera_that_stops_unsubscribes_its_slot() {
        let mut v = with_lines(1, false);
        let mut r = roster(json!({"8:a": person("Ada", Some(11), None)}));
        v.plan(&r, "8:me", |_| 0, |_| {});
        r.apply(&json!({"type": "Delta", "participants": {"8:a": {
            "version": 2, "state": "active", "details": {"displayName": "Ada"},
            "endpoints": {"ep": {"call": {"mediaStreams": []}}}}}}));
        let asked = v.plan(&r, "8:me", |_| 0, |_| {});
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].source, None);
        assert_eq!(v.slot_of("8:a"), None);
    }

    #[test]
    fn someone_else_s_share_is_subscribed_and_named() {
        let mut v = with_lines(0, true);
        let r = roster(json!({
            "8:me": person("Me", None, Some(20)),
            "8:a": person("Ada", None, Some(21)),
        }));
        let asked = v.plan(&r, "8:me", |_| 0, |_| {});
        assert_eq!(asked.len(), 1);
        assert_eq!((asked[0].mid.as_str(), asked[0].source), ("2", Some(21)));
        assert_eq!(asked[0].limits, SCREEN_LIMITS);
        assert_eq!(v.presenter_changed(&r, "8:me").as_deref(), Some("Ada"));
        assert_eq!(v.presenter_changed(&r, "8:me"), None);
        let nobody = roster(json!({"8:me": person("Me", None, None)}));
        assert_eq!(v.presenter_changed(&nobody, "8:me").as_deref(), Some(""));
    }

    #[test]
    fn the_apply_link_falls_back_to_the_call_leg() {
        let mut links = Links::default();
        assert_eq!(apply_link(&links), None);
        links
            .0
            .insert("callLeg".into(), "https://cc.example.invalid/leg/7/".into());
        assert_eq!(
            apply_link(&links).as_deref(),
            Some("https://cc.example.invalid/leg/7/applyChannelParameters")
        );
    }
}
