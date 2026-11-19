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
//! what the speaker plays, as `call join --mic --speaker` runs it (`KELMA_NO_AEC=1` turns it off,
//! like `--no-aec`). `KELMA_CALL_DUMP=<dir>` writes every raw calling push there.

use super::{
    CallDriver, CallHandle, Control, Device, Devices, Event, EventSink, JoinOptions, JoinTarget,
    Link, Person, Phase, live_phase, recording_state,
};
use anyhow::Context;
use calling::audio::{
    AudioProcessor, Capture, Playback, ProcessingOptions, echo_cancelled, spawn_capture_encoder,
    spawn_decoder_playback,
};
use calling::invitation::{CallEvent, NgcPush, classify};
use calling::media::{AudioSender, MediaEvents, MediaOptions, MediaSession, MediaState};
use calling::ngc_trouter::{CallPush, NgcEvent, NgcTrouter};
use calling::signaling::meeting::{MeetingTarget, meeting_from_csa_chat, meeting_from_join_url};
use calling::signaling::roster::{Presence, roster_snapshot};
use calling::signaling::{CallSession, Controls, NgcClient, Roster};
use calling::{CallInvitation, IceServer, Links, Modality, OpusPacket, SdpBlob};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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

    pub async fn ngc_client(&self) -> anyhow::Result<NgcClient> {
        if let Some(n) = self.ngc() {
            return Ok(n);
        }
        let mut n = NgcClient::discover(self.client.clone())
            .await
            .context("calling endpoints")?;
        if dump_dir().is_some() {
            n = n.with_observer(Arc::new(dump_exchange));
        }
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
        dump_push(&p);
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
        }
    }
}

/// `KELMA_CALL_DUMP=<dir>`: every raw calling push, and each SDP an incoming call exchanges, is
/// written there as it arrives (as `kelma-cli call listen --dump` does), for protocol debugging.
/// The files hold real ids, names and addresses: keep the directory out of the repo.
fn dump_dir() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = PathBuf::from(std::env::var_os("KELMA_CALL_DUMP").filter(|v| !v.is_empty())?);
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                tracing::info!(dir = %dir.display(), "dumping calling pushes");
                Some(dir)
            }
            Err(e) => {
                tracing::warn!(%e, "KELMA_CALL_DUMP: cannot create the directory");
                None
            }
        }
    })
    .as_deref()
}

/// `<dir>/<unix ms>-<seq>-<name>.<ext>`: sorts in arrival order.
fn dump_path(dir: &Path, name: &str, ext: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{ms}-{n:04}-{name}.{ext}"))
}

/// Writes one push as `{url, body}` JSON into `dir`.
fn write_push(dir: &Path, p: &CallPush) -> std::io::Result<PathBuf> {
    let path = dump_path(dir, "push", "json");
    let doc = serde_json::json!({"url": p.url, "body": p.body});
    std::fs::write(&path, serde_json::to_vec_pretty(&doc)?)?;
    Ok(path)
}

fn dump_push(p: &CallPush) {
    if let Some(dir) = dump_dir()
        && let Err(e) = write_push(dir, p)
    {
        tracing::warn!(%e, "KELMA_CALL_DUMP: push not written");
    }
}

/// Writes what answering a call reports (`KELMA_CALL_DUMP`): a step or a blob as `.txt`, an SDP
/// as `.sdp` (see `calling::incoming::Observe`).
pub(crate) fn dump_answer(name: &str, text: &str) {
    if let Some(dir) = dump_dir() {
        let ext = if name == "step" || name.ends_with("-blob") {
            "txt"
        } else {
            "sdp"
        };
        let _ = std::fs::write(dump_path(dir, name, ext), text);
    }
}

/// `{op, method, url, status, request, response}` of one request to the conversation service.
fn exchange_doc(e: &calling::signaling::Exchange<'_>) -> serde_json::Value {
    serde_json::json!({
        "op": e.op,
        "method": e.method,
        "url": e.url,
        "status": e.status,
        "request": e.request,
        "response": serde_json::from_str::<serde_json::Value>(e.response)
            .unwrap_or_else(|_| serde_json::Value::String(e.response.to_owned())),
    })
}

/// Writes every request kelma sends to the conversation service, with its answer
/// (`KELMA_CALL_DUMP`).
fn dump_exchange(e: &calling::signaling::Exchange<'_>) {
    if let Some(dir) = dump_dir() {
        let path = dump_path(dir, &format!("ngc-{}", e.op), "json");
        if let Ok(bytes) = serde_json::to_vec_pretty(&exchange_doc(e)) {
            let _ = std::fs::write(path, bytes);
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
        // An answered 1:1 call has its controller from the attach answer; a placed one may learn
        // it from its conversation push.
        let controller = links
            .get(calling::signaling::bodies::CONVERSATION_CONTROLLER)
            .unwrap_or_default()
            .to_owned();
        let mut call = Call {
            hub,
            ngc,
            s: session,
            links,
            controller,
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

/// Echo cancellation runs unless `KELMA_NO_AEC` is set to anything but `0` (the GUI's
/// `call join --no-aec`, for comparing both live) or the user turned it off in Settings ›
/// Devices; noise suppression follows its own switch there.
fn processing_wanted() -> ProcessingOptions {
    let prefs = session::Paths::new()
        .map(|p| crate::backend::settings::Local::load(&p.config_dir).devices)
        .unwrap_or_default();
    processing_from(&prefs, std::env::var_os("KELMA_NO_AEC").as_deref())
}

fn processing_from(
    prefs: &devices::DevicePrefs,
    no_aec: Option<&std::ffi::OsStr>,
) -> ProcessingOptions {
    ProcessingOptions {
        echo_cancellation: prefs.echo_cancellation && aec_wanted_by(no_aec),
        noise_suppression: prefs.noise_suppression,
    }
}

/// The conversation controller a 1:1 call's conversation push names (`conversationController`).
/// A 1:1 call starts without one, and the controls (mute) need it: `updateEndpointState` sits
/// under it. Only an https URL counts.
fn conversation_controller(body: &serde_json::Value) -> Option<String> {
    body.get("conversationController")
        .and_then(serde_json::Value::as_str)
        .filter(|u| u.starts_with("https://"))
        .map(str::to_owned)
}

fn aec_wanted_by(no_aec: Option<&std::ffi::OsStr>) -> bool {
    no_aec.is_none_or(|v| v.is_empty() || v == "0")
}

/// The microphone path of every call (meetings and adopted 1:1 calls), as `kelma-cli call join
/// --mic --speaker` runs it: the capture through AEC3, noise suppression and AGC2, then Opus.
/// With either switch on, the speaker must report each played frame to the returned reference
/// (its delay to the microphone is estimated inside from the devices' measured latency).
fn mic_chain<S: calling::audio::FrameSource + 'static>(
    capture: S,
    options: ProcessingOptions,
) -> (
    mpsc::Receiver<OpusPacket>,
    Option<calling::audio::EchoReference>,
) {
    if options.echo_cancellation || options.noise_suppression {
        let (mic, reference) = echo_cancelled(capture, AudioProcessor::with_options(options));
        (spawn_capture_encoder(mic), Some(reference))
    } else {
        (spawn_capture_encoder(capture), None)
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
        let (mut encoded, reference) = mic_chain(capture, processing_wanted());
        let out = match reference {
            Some(r) => Playback::start_with_tap(speaker, move |f| r.push(f)),
            None => Playback::start(speaker),
        }
        .context("speaker")?;
        self.play_into(out);
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

/// Who is heard: the participants whose audio `sourceId`s the latest packets name, and the order
/// people last spoke in (the floating call shows the latest speakers first).
#[derive(Debug, Default)]
struct Talk {
    now: std::collections::HashSet<String>,
    last: std::collections::HashMap<String, u64>,
    tick: u64,
}

impl Talk {
    /// Takes the sources heard now; true when who speaks changed.
    fn heard(&mut self, roster: &Roster, sources: &[u32]) -> bool {
        let now: std::collections::HashSet<String> = roster
            .participants()
            .iter()
            .filter(|p| {
                p.endpoints
                    .iter()
                    .any(|e| e.audio_sources.iter().any(|s| sources.contains(s)))
            })
            .map(|p| p.mri.clone())
            .collect();
        if now == self.now {
            return false;
        }
        self.tick += 1;
        for m in &now {
            self.last.insert(m.clone(), self.tick);
        }
        self.now = now;
        true
    }
}

fn people(roster: &Roster, me: &str, talk: &Talk) -> Vec<Person> {
    roster
        .participants()
        .iter()
        .map(|p| Person {
            mri: p.mri.clone(),
            speaking: talk.now.contains(&p.mri),
            spoke: talk.last.get(&p.mri).copied().unwrap_or(0),
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
        let mut speakers_rx = events.speakers;
        let mut talk = Talk::default();
        let mut speakers_open = true;
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
                heard = speakers_rx.changed(), if speakers_open => match heard {
                    Ok(()) => {
                        let sources = speakers_rx.borrow_and_update().clone();
                        roster_changed = talk.heard(&self.roster, &sources);
                    }
                    Err(_) => speakers_open = false,
                },
                st = state_rx.recv() => match st {
                    Some(MediaState::Failed | MediaState::Closed) | None => {
                        break Some("The connection was lost.".into());
                    }
                    Some(st) => self.link = link_of(st),
                },
                p = pushes.recv() => {
                    let Some(p) = p else { break Some(self.ended_text()) };
                    if self.controller.is_empty()
                        && let Some(c) = conversation_controller(&p.body)
                    {
                        self.controller = c;
                    }
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
                                speakers_rx = ev.speakers;
                                speakers_open = true;
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
                sink.send(Event::Roster(people(&self.roster, &me, &talk)));
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
    fn a_direct_call_learns_its_controller_from_the_conversation_push() {
        let push = json!({
            "conversationController": "https://api.example.test/api/v2/cc/conv-1",
            "participants": ["8:orgid:a"],
        });
        assert_eq!(
            conversation_controller(&push).as_deref(),
            Some("https://api.example.test/api/v2/cc/conv-1")
        );
        assert_eq!(conversation_controller(&json!({"participants": []})), None);
        assert_eq!(
            conversation_controller(&json!({"conversationController": "http://x.invalid"})),
            None
        );
    }

    #[test]
    fn echo_cancellation_is_on_unless_opted_out() {
        use std::ffi::OsStr;
        assert!(aec_wanted_by(None));
        assert!(aec_wanted_by(Some(OsStr::new(""))));
        assert!(aec_wanted_by(Some(OsStr::new("0"))));
        assert!(!aec_wanted_by(Some(OsStr::new("1"))));
    }

    #[test]
    fn the_device_switches_shape_the_mic_processing() {
        use std::ffi::OsStr;
        let on = devices::DevicePrefs::default();
        assert_eq!(processing_from(&on, None), ProcessingOptions::default());
        // KELMA_NO_AEC still turns echo cancellation off, noise suppression stays.
        let p = processing_from(&on, Some(OsStr::new("1")));
        assert!(!p.echo_cancellation && p.noise_suppression);
        let off = devices::DevicePrefs {
            noise_suppression: false,
            echo_cancellation: false,
            ..Default::default()
        };
        assert_eq!(
            processing_from(&off, None),
            ProcessingOptions {
                echo_cancellation: false,
                noise_suppression: false
            }
        );
    }

    #[tokio::test]
    async fn the_mic_chain_runs_through_the_echo_canceller() {
        use calling::audio::{EchoReference, ToneSource};
        // With AEC the speaker gets a reference to feed; it accepts a played 20 ms frame.
        let (mut packets, reference) =
            mic_chain(ToneSource::endless(440.0), ProcessingOptions::default());
        let reference: EchoReference = reference.expect("AEC stage in the chain");
        reference.push(&[0i16; calling::FRAME_SAMPLES]);
        assert!(packets.recv().await.is_some(), "the chain still encodes");
        // With both switches off there is no processing stage and nothing to feed.
        let off = ProcessingOptions {
            echo_cancellation: false,
            noise_suppression: false,
        };
        let (mut raw, none) = mic_chain(ToneSource::endless(440.0), off);
        assert!(none.is_none());
        assert!(raw.recv().await.is_some());
    }

    #[test]
    fn an_exchange_dump_has_the_request_and_the_parsed_answer() {
        let request = json!({"attach": {"requireMediaContent": true}});
        let doc = exchange_doc(&calling::signaling::Exchange {
            op: "attach",
            method: "POST",
            url: "https://cc.example.invalid/attach",
            request: Some(&request),
            status: 200,
            response: r#"{"callInvitation": {"links": {}}}"#,
        });
        assert_eq!(doc["request"]["attach"]["requireMediaContent"], true);
        assert_eq!(doc["response"]["callInvitation"]["links"], json!({}));
        let failed = exchange_doc(&calling::signaling::Exchange {
            op: "attach",
            method: "POST",
            url: "https://cc.example.invalid/attach",
            request: None,
            status: 0,
            response: "connection reset",
        });
        assert_eq!(failed["response"], "connection reset");
        assert_eq!(failed["request"], json!(null));
    }

    #[test]
    fn a_dumped_push_is_its_url_and_body_in_arrival_order() {
        let dir = std::env::temp_dir().join(format!("kelma-dump-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let push = |n: i64| CallPush {
            id: n,
            url: format!("https://trouter.example.invalid/v4/f/x/NGCallManagerWin/{n}"),
            headers: Default::default(),
            body: json!({"callNotification": {"mediaContent": null}}),
        };
        let first = write_push(&dir, &push(1)).unwrap();
        let second = write_push(&dir, &push(2)).unwrap();
        assert!(first < second, "names sort in arrival order");
        let doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&first).unwrap()).unwrap();
        assert_eq!(doc["url"], push(1).url);
        assert_eq!(doc["body"]["callNotification"]["mediaContent"], json!(null));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_live_driver_is_native() {
        // Built only: joining or listing devices would reach the network and the sound cards.
        let d = LiveDriver::new(Hub::new(Client::offline("t").unwrap()));
        assert!(d.native());
    }

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
        let p = people(&r, "8:orgid:me", &Talk::default());
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
