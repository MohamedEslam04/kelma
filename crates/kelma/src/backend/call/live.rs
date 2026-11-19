//! The live call driver (feature `calls`): kelma's native calling engine behind the call window.
//!
//! A port of `kelma-cli call join` (`crates/kelma-cli/src/call.rs`), which was verified live:
//! discover, TURN, the calling trouter, a media session and the join; the lobby, whose admit
//! re-offer restarts ICE and is answered on a fresh media session (the sender follows a watch so
//! the audio pipeline survives the swap); keepAlive, the roster, the controls and leave.
//!
//! [`Hub`] owns the one calling trouter of the app (per endpoint id). It starts at sign-in, so
//! incoming call invitations always arrive, and routes every push: the active call's callbacks to
//! it, invitations and every other push to the ringing flows' subscriptions.
//!
//! Audio is the microphone through echo cancellation (AEC3, noise suppression, AGC2), fed by
//! what the speaker plays, as `call join --mic --speaker` runs it.

// Unit tests run the fake driver, so the live one is only built, not run, under `cfg(test)`.
#![cfg_attr(test, allow(dead_code))]

use super::{
    CallDriver, CallHandle, Control, Device, Devices, Event, EventSink, JoinOptions, JoinTarget,
    Link, Person, Phase, live_phase, recording_state,
};
use anyhow::Context;
use calling::audio::{
    AudioProcessor, Capture, Playback, echo_cancelled, spawn_capture_encoder,
    spawn_decoder_playback,
};
use calling::invitation::{CallEvent, NgcPush, classify};
use calling::media::{AudioSender, MediaEvents, MediaOptions, MediaSession, MediaState};
use calling::ngc_trouter::{CallPush, NgcEvent, NgcTrouter};
use calling::signaling::meeting::{MeetingTarget, meeting_from_csa_chat, meeting_from_join_url};
use calling::signaling::roster::{Presence, roster_snapshot};
use calling::signaling::{CallSession, Controls, NgcClient, Roster};
use calling::{CallInvitation, IceServer, Links, Modality, OpusPacket, SdpBlob};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use teams_proto::Client;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

const CSA_ME: &str = "https://teams.microsoft.com/api/csa/api/v1/teams/users/me?isPrefetch=false&enableMembershipSummary=true";
const MODALITIES: [Modality; 1] = [Modality::Audio];
/// Incoming packets waiting for the decoder: about five seconds.
const PLAY_QUEUE: usize = 256;

/// A calling-trouter endpoint id: stable per install and distinct from the chat trouter's (as
/// `kelma-cli`'s `calling_epid`).
fn calling_epid(chat_epid: &str) -> String {
    let tail = chat_epid.get(8..).unwrap_or(chat_epid);
    format!("ca11ca11{tail}")
}

/// The calling trouter and the conversation service client, shared by every call of the app.
pub struct Hub {
    client: Client,
    trouter: Mutex<Option<NgcTrouter>>,
    ngc: Mutex<Option<NgcClient>>,
    /// The active call's session id and where its callbacks go.
    active: Mutex<Option<(String, mpsc::UnboundedSender<CallPush>)>>,
    invitations: Mutex<Vec<mpsc::UnboundedSender<Box<CallInvitation>>>>,
    unmatched: Mutex<Vec<mpsc::UnboundedSender<CallPush>>>,
}

impl Hub {
    pub fn new(client: Client) -> Arc<Hub> {
        Arc::new(Hub {
            client,
            trouter: Mutex::new(None),
            ngc: Mutex::new(None),
            active: Mutex::new(None),
            invitations: Mutex::new(Vec::new()),
            unmatched: Mutex::new(Vec::new()),
        })
    }

    /// Starts the calling trouter (once) and discovers the conversation service. Needs a tokio
    /// runtime.
    pub fn start(self: &Arc<Self>) {
        let mut slot = self.trouter.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let (trouter, mut rx) =
            NgcTrouter::spawn(self.client.clone(), calling_epid(self.client.epid()));
        *slot = Some(trouter);
        drop(slot);
        let hub = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                let Some(h) = hub.upgrade() else { return };
                match ev {
                    NgcEvent::Push(p) => h.route(p),
                    NgcEvent::Connected { .. } => tracing::debug!("calling trouter registered"),
                    // A reconnect changes the surl, which a running call's links were built on.
                    NgcEvent::Disconnected(r) => tracing::warn!(%r, "calling trouter disconnected"),
                }
            }
        });
        let hub = self.clone();
        tokio::spawn(async move {
            if let Err(e) = hub.ngc_client().await {
                tracing::warn!(%e, "calling endpoints not discovered");
            }
        });
    }

    pub fn ngc(&self) -> Option<NgcClient> {
        self.ngc.lock().unwrap().clone()
    }

    async fn ngc_client(&self) -> anyhow::Result<NgcClient> {
        if let Some(n) = self.ngc() {
            return Ok(n);
        }
        let n = NgcClient::discover(self.client.clone())
            .await
            .context("calling endpoints")?;
        *self.ngc.lock().unwrap() = Some(n.clone());
        Ok(n)
    }

    pub fn surl(&self) -> Option<String> {
        self.trouter.lock().unwrap().as_ref()?.surl()
    }

    async fn connected_surl(self: &Arc<Self>) -> Option<String> {
        self.start();
        let mut w = self.trouter.lock().unwrap().as_ref()?.surl_watch();
        tokio::time::timeout(Duration::from_secs(30), w.wait_for(Option::is_some))
            .await
            .ok()?
            .ok()?
            .clone()
    }

    /// Sends one push where it belongs.
    pub fn route(&self, p: CallPush) {
        match classify(&p.url, &p.body) {
            NgcPush::Invitation(inv) => self
                .invitations
                .lock()
                .unwrap()
                .retain(|tx| tx.send(inv.clone()).is_ok()),
            NgcPush::Call { session_id, .. } => {
                let active = self.active.lock().unwrap().clone();
                match active {
                    Some((id, tx)) if id == session_id => {
                        let _ = tx.send(p);
                    }
                    _ => self.unmatched(p),
                }
            }
            NgcPush::Other { .. } => self.unmatched(p),
        }
    }

    fn unmatched(&self, p: CallPush) {
        self.unmatched
            .lock()
            .unwrap()
            .retain(|tx| tx.send(p.clone()).is_ok());
    }

    pub fn subscribe_invitations(&self) -> mpsc::UnboundedReceiver<Box<CallInvitation>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.invitations.lock().unwrap().push(tx);
        rx
    }

    pub fn subscribe_unmatched(&self) -> mpsc::UnboundedReceiver<CallPush> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.unmatched.lock().unwrap().push(tx);
        rx
    }

    /// Routes the callbacks of `session_id` to the returned receiver from now on.
    fn set_active(&self, session_id: &str) -> mpsc::UnboundedReceiver<CallPush> {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.active.lock().unwrap() = Some((session_id.to_owned(), tx));
        rx
    }

    fn clear_active(&self, session_id: &str) {
        let mut a = self.active.lock().unwrap();
        if a.as_ref().is_some_and(|(id, _)| id == session_id) {
            *a = None;
        }
    }

    async fn ice_servers(&self, region: &str) -> Vec<IceServer> {
        match calling::relay::resolve_ice_servers(&self.client, region, None).await {
            Ok((ice, _)) => ice,
            Err(e) => {
                tracing::warn!(%e, "TURN unavailable; host candidates only");
                vec![]
            }
        }
    }

    /// The meeting a call window target names.
    async fn resolve(&self, target: JoinTarget) -> anyhow::Result<MeetingTarget> {
        match target {
            JoinTarget::Link(url) => {
                meeting_from_join_url(&url).context("This isn't a Teams meeting link.")
            }
            JoinTarget::Code { id, passcode } => ::calendar::join_by_id_url(&id, &passcode)
                .and_then(|u| meeting_from_join_url(&u))
                .context("That doesn't look like a meeting ID."),
            JoinTarget::Chat(thread) => {
                let v = self
                    .client
                    .json("csa", reqwest::Method::GET, CSA_ME, None)
                    .await?;
                v["chats"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|c| c["id"].as_str() == Some(thread.as_str()))
                    .and_then(meeting_from_csa_chat)
                    .context("This chat has no meeting to join.")
            }
            JoinTarget::Target(t) => Ok(*t),
        }
    }
}

/// The driver the app runs with.
pub struct LiveDriver {
    hub: Arc<Hub>,
}

impl LiveDriver {
    pub fn new(hub: Arc<Hub>) -> Self {
        LiveDriver { hub }
    }
}

/// Microphones and speakers from `crates/devices`, the one picked in Settings first (else the
/// system default).
fn list_devices() -> Devices {
    let prefs = session::Paths::new()
        .map(|p| crate::backend::settings::Local::load(&p.config_dir).devices)
        .unwrap_or_default();
    let inputs = devices::audio_inputs().unwrap_or_default();
    let outputs = devices::audio_outputs().unwrap_or_default();
    let index = |list: &[devices::AudioDevice], chosen: Option<&devices::AudioDevice>| {
        chosen
            .and_then(|c| list.iter().position(|d| d.id == c.id))
            .or_else(|| list.iter().position(|d| d.is_default))
            .unwrap_or(0)
    };
    let to = |list: &[devices::AudioDevice]| {
        list.iter()
            .map(|d| Device {
                id: Some(d.id.clone()),
                name: d.name.clone(),
            })
            .collect()
    };
    Devices {
        mic: index(&inputs, prefs.find_microphone(&inputs)),
        speaker: index(&outputs, prefs.find_speaker(&outputs)),
        mics: to(&inputs),
        speakers: to(&outputs),
    }
}

struct LiveHandle(mpsc::UnboundedSender<Control>);

impl CallHandle for LiveHandle {
    fn control(&self, c: Control) {
        let _ = self.0.send(c);
    }
}

impl CallDriver for LiveDriver {
    fn devices(&self) -> Devices {
        list_devices()
    }

    fn join(&self, target: JoinTarget, opts: JoinOptions, sink: EventSink) -> Box<dyn CallHandle> {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run_meeting(self.hub.clone(), target, opts, sink, rx));
        Box::new(LiveHandle(tx))
    }
}

async fn run_meeting(
    hub: Arc<Hub>,
    target: JoinTarget,
    opts: JoinOptions,
    sink: EventSink,
    mut ctl: mpsc::UnboundedReceiver<Control>,
) {
    sink.send(Event::Connecting);
    let setup = join_meeting(hub.clone(), target, opts.clone());
    tokio::pin!(setup);
    // Controls pressed while joining wait; Leave (or closing the window) gives up the join.
    let mut early = Vec::new();
    let joined = loop {
        tokio::select! {
            r = &mut setup => break r,
            c = ctl.recv() => match c {
                None | Some(Control::Leave) => return,
                Some(c) => early.push(c),
            },
        }
    };
    match joined {
        Ok((call, events, pushes)) => call.run(events, pushes, opts, sink, ctl, early).await,
        Err(e) => {
            tracing::warn!("join failed: {e:#}");
            sink.send(Event::Error(format!("Couldn't join: {e:#}")));
            sink.send(Event::Ended("Couldn't join the meeting.".into()));
        }
    }
}

type Joined = (Call, MediaEvents, mpsc::UnboundedReceiver<CallPush>);

/// Discover, TURN, trouter, media offer, join (the first half of `call join`).
async fn join_meeting(
    hub: Arc<Hub>,
    target: JoinTarget,
    opts: JoinOptions,
) -> anyhow::Result<Joined> {
    let target = hub.resolve(target).await?;
    let ngc = hub.ngc_client().await?;
    let surl = hub
        .connected_surl()
        .await
        .context("the calling connection didn't come up")?;
    let ice = hub.ice_servers(&ngc.endpoints().region).await;
    let (media, events) = MediaSession::with_options(ice.clone(), MediaOptions::default()).await?;
    let offer = media.create_offer().await?;
    let me = hub.client.self_mri().await?;
    let s = CallSession::new(&surl, &me, &opts.name);
    // Callbacks can arrive before the join answers.
    let pushes = hub.set_active(s.callbacks.session_id());
    let conv = match ngc
        .join(&s, &target, &SdpBlob::ngc(offer), &MODALITIES)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            hub.clear_active(s.callbacks.session_id());
            let _ = media.close().await;
            return Err(e.into());
        }
    };
    let mut roster = Roster::default();
    if let Some(snapshot) = roster_snapshot(&conv.raw) {
        roster.apply_snapshot(snapshot);
    }
    let mut answered = false;
    if let Some(ans) = find_blob(&conv.raw) {
        match ans.decode() {
            Ok(sdp) => match media.set_answer(&sdp).await {
                Ok(()) => answered = true,
                Err(e) => tracing::warn!(%e, "inline media answer refused"),
            },
            Err(e) => tracing::warn!(%e, "undecodable inline answer"),
        }
    }
    let call = Call {
        hub,
        ngc,
        s,
        links: conv.links.clone(),
        controller: conv.controller.clone(),
        media,
        ice,
        answered,
        roster,
        controls: Controls::default(),
        meeting: true,
        keep_alive: None,
        link: Link::New,
    };
    Ok((call, events, pushes))
}

/// Runs a connected direct call handed over by the ringing flows.
pub fn adopt(
    hub: Arc<Hub>,
    session: CallSession,
    links: Links,
    media: MediaSession,
    events: MediaEvents,
    opts: JoinOptions,
    sink: EventSink,
) -> Box<dyn CallHandle> {
    let (tx, rx) = mpsc::unbounded_channel();
    let pushes = hub.set_active(session.callbacks.session_id());
    tokio::spawn(async move {
        let ngc = match hub.ngc_client().await {
            Ok(n) => n,
            Err(e) => {
                hub.clear_active(session.callbacks.session_id());
                let _ = media.close().await;
                sink.send(Event::Error(format!("{e:#}")));
                sink.send(Event::Ended("The call ended.".into()));
                return;
            }
        };
        let mut call = Call {
            hub,
            ngc,
            s: session,
            links,
            // A 1:1 call names no conversation controller; the controls use the call's links.
            controller: String::new(),
            media,
            ice: vec![],
            answered: true,
            roster: Roster::default(),
            controls: Controls::default(),
            meeting: false,
            keep_alive: None,
            link: Link::Connected,
        };
        call.start_keep_alive(None);
        call.run(events, pushes, opts, sink, rx, vec![]).await;
    });
    Box::new(LiveHandle(tx))
}

/// The first `{contentType, blob}` object anywhere in `v` (a join answer may carry the media
/// answer inline). As in `kelma-cli`.
fn find_blob(v: &serde_json::Value) -> Option<SdpBlob> {
    use serde_json::Value;
    match v {
        Value::Object(m) => {
            if let (Some(ct), Some(b)) = (
                m.get("contentType").and_then(Value::as_str),
                m.get("blob").and_then(Value::as_str),
            ) && !b.is_empty()
            {
                return Some(SdpBlob {
                    content_type: ct.to_owned(),
                    blob: b.to_owned(),
                });
            }
            m.values().find_map(find_blob)
        }
        Value::Array(a) => a.iter().find_map(find_blob),
        _ => None,
    }
}

/// The audio of one call: microphone to the current media sender, incoming packets to the
/// speaker. Restartable for a device change; the media session can be swapped under it.
struct Audio {
    sender: watch::Receiver<AudioSender>,
    muted: Arc<AtomicBool>,
    play: watch::Sender<Option<mpsc::Sender<OpusPacket>>>,
    send_task: Option<JoinHandle<()>>,
    playback: Option<JoinHandle<calling::Result<()>>>,
}

impl Audio {
    /// (Re)opens the devices: the mic through the echo canceller, whose reference is what the
    /// speaker plays. Without a microphone the speaker still plays.
    fn start(&mut self, mic: Option<&str>, speaker: Option<&str>) -> anyhow::Result<()> {
        self.stop();
        let capture = match Capture::start(mic) {
            Ok(c) => c,
            Err(e) => {
                let out = Playback::start(speaker).context("speaker")?;
                self.play_into(out);
                return Err(anyhow::Error::from(e).context("microphone"));
            }
        };
        let (mic, reference) = echo_cancelled(capture, AudioProcessor::new());
        let out =
            Playback::start_with_tap(speaker, move |f| reference.push(f)).context("speaker")?;
        self.play_into(out);
        let mut encoded = spawn_capture_encoder(mic);
        let (sender, muted) = (self.sender.clone(), self.muted.clone());
        self.send_task = Some(tokio::spawn(async move {
            while let Some(p) = encoded.recv().await {
                // Muting is also local: the packets stop here.
                if muted.load(Ordering::Relaxed) {
                    continue;
                }
                let s = sender.borrow().clone();
                let _ = s.send(p).await;
            }
        }));
        Ok(())
    }

    fn play_into(&mut self, out: Playback) {
        let (tx, rx) = mpsc::channel(PLAY_QUEUE);
        self.playback = Some(spawn_decoder_playback(rx, out));
        self.play.send_replace(Some(tx));
    }

    fn stop(&mut self) {
        // The playback ends once its queue closes; the encoder once its reader is gone.
        self.play.send_replace(None);
        self.playback.take();
        if let Some(t) = self.send_task.take() {
            t.abort();
        }
    }
}

/// Moves one media session's incoming packets to whatever playback is current.
fn forward(
    play: watch::Receiver<Option<mpsc::Sender<OpusPacket>>>,
    mut incoming: mpsc::Receiver<OpusPacket>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(p) = incoming.recv().await {
            let tx = play.borrow().clone();
            if let Some(tx) = tx {
                let _ = tx.send(p).await;
            }
        }
    })
}

fn link_of(st: MediaState) -> Link {
    match st {
        MediaState::New => Link::New,
        MediaState::Connecting => Link::Connecting,
        MediaState::Connected => Link::Connected,
        MediaState::Disconnected | MediaState::Failed | MediaState::Closed => Link::Disconnected,
    }
}

fn people(roster: &Roster, me: &str) -> Vec<Person> {
    roster
        .participants()
        .iter()
        .map(|p| Person {
            name: if p.display_name.is_empty() {
                "Unknown".into()
            } else {
                p.display_name.clone()
            },
            muted: p.muted(),
            hand: p.hand.is_some(),
            lobby: p.presence == Presence::Lobby,
            me: p.mri == me,
            role: p.role.map(|r| r.name().to_owned()).unwrap_or_default(),
        })
        .collect()
}

/// One joined call.
struct Call {
    hub: Arc<Hub>,
    ngc: NgcClient,
    s: CallSession,
    links: Links,
    controller: String,
    media: MediaSession,
    ice: Vec<IceServer>,
    answered: bool,
    roster: Roster,
    controls: Controls,
    meeting: bool,
    keep_alive: Option<JoinHandle<()>>,
    link: Link,
}

impl Call {
    fn start_keep_alive(&mut self, interval_s: Option<u64>) {
        if self.keep_alive.is_some() {
            return;
        }
        let (ngc, s, l) = (self.ngc.clone(), self.s.clone(), self.links.clone());
        self.keep_alive = Some(tokio::spawn(async move {
            if let Err(e) = ngc.run_keep_alive(&s, &l, interval_s).await {
                tracing::warn!(%e, "keepAlive stopped");
            }
        }));
    }

    async fn apply_answer(&mut self, answer: &SdpBlob) {
        if self.answered {
            return;
        }
        match answer.decode() {
            Ok(sdp) => match self.media.set_answer(&sdp).await {
                Ok(()) => self.answered = true,
                Err(e) => tracing::warn!(%e, "media answer refused"),
            },
            Err(e) => tracing::warn!(%e, "undecodable media answer"),
        }
    }

    async fn control(&mut self, c: Control, audio: &mut Audio, sink: &EventSink) {
        let (s, links, controller) = (&self.s, &self.links, self.controller.as_str());
        match c {
            Control::Mute(m) => {
                audio.muted.store(m, Ordering::Relaxed);
                if let Err(e) = self
                    .ngc
                    .set_muted(s, links, controller, &mut self.controls, m)
                    .await
                {
                    tracing::warn!(%e, "mute state not published");
                    sink.send(Event::Error(
                        "Others may not see your mute state change.".into(),
                    ));
                }
            }
            Control::RaiseHand => {
                if let Err(e) = self
                    .ngc
                    .raise_hand(s, links, controller, &mut self.controls)
                    .await
                {
                    tracing::warn!(%e, "raise hand failed");
                    sink.send(Event::Error("Couldn't raise your hand.".into()));
                    sink.send(Event::Hand(false));
                }
            }
            Control::LowerHand => {
                let own = self.roster.hand_of(&s.me.mri).map(|h| h.state_id.clone());
                if let Err(e) = self
                    .ngc
                    .lower_hand(s, links, controller, &mut self.controls, own.as_deref())
                    .await
                {
                    tracing::warn!(%e, "lower hand failed");
                    sink.send(Event::Error("Couldn't lower your hand.".into()));
                    sink.send(Event::Hand(true));
                }
            }
            Control::Devices { mic, speaker } => {
                if let Err(e) = audio.start(mic.as_deref(), speaker.as_deref()) {
                    sink.send(Event::Error(format!("Audio device: {e:#}")));
                }
            }
            Control::Leave => {}
        }
    }

    /// The rest of `call join`: audio, then controls, media state and callbacks until the call
    /// ends or we leave.
    async fn run(
        mut self,
        events: MediaEvents,
        mut pushes: mpsc::UnboundedReceiver<CallPush>,
        opts: JoinOptions,
        sink: EventSink,
        mut ctl: mpsc::UnboundedReceiver<Control>,
        early: Vec<Control>,
    ) {
        let (sender_tx, sender_rx) = watch::channel(self.media.audio_sender());
        let (play_tx, play_rx) = watch::channel(None);
        let mut audio = Audio {
            sender: sender_rx,
            muted: Arc::new(AtomicBool::new(!opts.mic_on)),
            play: play_tx,
            send_task: None,
            playback: None,
        };
        if let Err(e) = audio.start(opts.mic.as_deref(), opts.speaker.as_deref()) {
            sink.send(Event::Error(format!("Audio device: {e:#}")));
        }
        let mut forwarders = vec![forward(play_rx.clone(), events.audio)];
        let mut state_rx = events.state;
        if !opts.mic_on {
            self.control(Control::Mute(true), &mut audio, &sink).await;
        }
        for c in early {
            self.control(c, &mut audio, &sink).await;
        }
        let me = self.s.me.mri.clone();
        let (mut in_lobby, mut was_in_call) = (false, false);
        let mut shown: Option<Phase> = None;
        let mut own_hand: Option<bool> = None;
        let ended: Option<String> = loop {
            let mut roster_changed = false;
            tokio::select! {
                c = ctl.recv() => match c {
                    None | Some(Control::Leave) => break None,
                    Some(c) => self.control(c, &mut audio, &sink).await,
                },
                st = state_rx.recv() => match st {
                    Some(MediaState::Failed | MediaState::Closed) | None => {
                        break Some("The connection was lost.".into());
                    }
                    Some(st) => self.link = link_of(st),
                },
                p = pushes.recv() => {
                    let Some(p) = p else { break Some(self.ended_text()) };
                    let NgcPush::Call { event, .. } = classify(&p.url, &p.body) else { continue };
                    match event {
                        CallEvent::Accepted { answer, links, keep_alive_interval_s, .. } => {
                            self.links.0.extend(links.0);
                            if let Some(a) = answer {
                                self.apply_answer(&a).await;
                            }
                            self.start_keep_alive(keep_alive_interval_s);
                        }
                        CallEvent::MediaAnswer { answer, links } => {
                            self.links.0.extend(links.0);
                            self.apply_answer(&answer).await;
                        }
                        CallEvent::MediaRenegotiation { offer, links } => {
                            let swapped = self.renegotiate(&offer, &links, &sender_tx, &sink).await;
                            if let Some(ev) = swapped {
                                state_rx = ev.state;
                                forwarders.push(forward(play_rx.clone(), ev.audio));
                            }
                            self.links.0.extend(links.0);
                            // Admission re-offers the media; we are out of the lobby.
                            in_lobby = false;
                        }
                        CallEvent::Rejected(r) | CallEvent::Ended(r) => {
                            tracing::info!(code = r.code, sub = r.sub_code, "call ended by the service");
                            break Some(self.ended_text());
                        }
                        CallEvent::RosterUpdate(delta) => roster_changed = self.roster.apply(&delta),
                        CallEvent::ConversationUpdate(body) => {
                            if let Some(snapshot) = roster_snapshot(&body) {
                                roster_changed = self.roster.apply_snapshot(snapshot);
                            }
                            if let Some(on) = recording_state(&body) {
                                sink.send(Event::Recording(on));
                            }
                        }
                        CallEvent::Progress { .. }
                        | CallEvent::MediaAcknowledgement { .. }
                        | CallEvent::Other { .. } => {}
                    }
                }
            }
            if roster_changed {
                sink.send(Event::Roster(people(&self.roster, &me)));
                if let Some(p) = self.roster.get(&me) {
                    in_lobby = p.presence == Presence::Lobby;
                    let hand = p.hand.is_some();
                    if own_hand.replace(hand).is_some_and(|h| h != hand) {
                        sink.send(Event::Hand(hand));
                    }
                }
            }
            let phase = live_phase(self.link, in_lobby && self.meeting, was_in_call);
            was_in_call |= phase == Phase::Connected;
            if shown != Some(phase) {
                shown = Some(phase);
                sink.send(match phase {
                    Phase::Lobby => Event::Lobby,
                    Phase::Connected => Event::Connected,
                    Phase::Reconnecting => Event::Reconnecting,
                    _ => Event::Connecting,
                });
            }
        };

        if ended.is_none() {
            let leave = if self.links.get("leave").is_some() || self.links.get("callLeg").is_some()
            {
                self.ngc.hang_up(&self.s, &self.links).await
            } else {
                self.ngc.leave_controller(&self.s, &self.controller).await
            };
            if let Err(e) = leave {
                tracing::warn!(%e, "leave failed");
            }
        }
        if let Some(k) = self.keep_alive.take() {
            k.abort();
        }
        audio.stop();
        let _ = self.media.close().await;
        for f in forwarders {
            f.abort();
        }
        self.hub.clear_active(self.s.callbacks.session_id());
        if let Some(reason) = ended {
            sink.send(Event::Ended(reason));
        }
    }

    fn ended_text(&self) -> String {
        if self.meeting {
            "The meeting ended.".into()
        } else {
            "The call ended.".into()
        }
    }

    /// Answers a re-offer. One that restarts ICE (the lobby's admission: new mids and ufrags) is
    /// refused by the connected session and answered on a fresh one, which replaces it; its
    /// events are returned so the caller follows them.
    async fn renegotiate(
        &mut self,
        offer: &SdpBlob,
        links: &Links,
        sender: &watch::Sender<AudioSender>,
        sink: &EventSink,
    ) -> Option<MediaEvents> {
        let sdp = match offer.decode() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(%e, "undecodable re-offer");
                return None;
            }
        };
        let mut fresh_events = None;
        let answer = match self.media.accept_offer(&sdp).await {
            Ok(a) => Ok(a),
            Err(e) => {
                tracing::info!(%e, "re-offer answered on a new media session");
                if self.ice.is_empty() {
                    self.ice = self.hub.ice_servers(&self.ngc.endpoints().region).await;
                }
                match MediaSession::with_options(self.ice.clone(), MediaOptions::default()).await {
                    Ok((fresh, ev)) => match fresh.accept_offer(&sdp).await {
                        Ok(ans) => {
                            let old = std::mem::replace(&mut self.media, fresh);
                            let _ = old.close().await;
                            sender.send_replace(self.media.audio_sender());
                            self.link = Link::New;
                            fresh_events = Some(ev);
                            Ok(ans)
                        }
                        Err(e) => {
                            let _ = fresh.close().await;
                            Err(e)
                        }
                    },
                    Err(e) => Err(e),
                }
            }
        };
        match answer {
            Ok(ans) => {
                if let Some(url) = links.get("mediaAnswer") {
                    match self
                        .ngc
                        .send_media_answer(&self.s, url, &MODALITIES, &SdpBlob::ngc(ans))
                        .await
                    {
                        // Teams ignored the first state update after admission (mautrix-teams),
                        // so publish the mute state again.
                        Ok(()) => {
                            let r = self
                                .ngc
                                .repeat_muted(
                                    &self.s,
                                    &self.links,
                                    &self.controller,
                                    &mut self.controls,
                                )
                                .await;
                            if let Err(e) = r {
                                tracing::warn!(%e, "mute state not repeated");
                            }
                        }
                        Err(e) => {
                            tracing::warn!(%e, "re-offer answer refused");
                            sink.send(Event::Error("The media update was refused.".into()));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(%e, "re-offer not answered");
                sink.send(Event::Error("Couldn't update the call's media.".into()));
            }
        }
        fresh_events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_calling_epid_is_derived_from_the_chat_one() {
        assert_eq!(calling_epid("abcdefgh-1234"), "ca11ca11-1234");
    }

    #[test]
    fn an_inline_answer_is_found_anywhere() {
        let v = json!({"a": [{"x": 1}, {"mediaContent": {"contentType": "application/sdp", "blob": "v=0"}}]});
        assert_eq!(find_blob(&v).unwrap().blob, "v=0");
        assert!(find_blob(&json!({"blob": ""})).is_none());
    }

    #[test]
    fn roster_people_mark_us_and_the_lobby() {
        let mut r = Roster::default();
        r.apply(&json!({"participants": {
            "8:orgid:me": {"version": 1, "state": "active", "details": {"displayName": "Me"},
                "endpoints": {"e": {"call": {}, "endpointState": {"state": {"isMuted": true}}}}},
            "8:orgid:g": {"version": 1, "state": "active", "details": {"displayName": ""},
                "endpoints": {"e": {"lobby": {}}}}
        }}));
        let p = people(&r, "8:orgid:me");
        let me = p.iter().find(|p| p.me).unwrap();
        assert!(me.muted && !me.lobby);
        let guest = p.iter().find(|p| !p.me).unwrap();
        assert_eq!((guest.name.as_str(), guest.lobby), ("Unknown", true));
    }

    #[test]
    fn pushes_route_to_the_active_call_or_the_subscribers() {
        let hub = Hub::new(Client::offline("t").unwrap());
        let mut unmatched = hub.subscribe_unmatched();
        let mut active = hub.set_active("sess-1");
        let push = |session: &str| CallPush {
            id: 1,
            url: format!("https://surl/callAgent/{session}/abcd1234/conversation/rosterUpdate/"),
            headers: Default::default(),
            body: json!({"participants": {}}),
        };
        hub.route(push("sess-1"));
        hub.route(push("sess-2"));
        assert!(active.try_recv().is_ok());
        assert!(active.try_recv().is_err());
        assert!(unmatched.try_recv().unwrap().url.contains("sess-2"));
        hub.clear_active("sess-1");
        hub.route(push("sess-1"));
        assert!(unmatched.try_recv().is_ok(), "no active call: unmatched");
    }
}
