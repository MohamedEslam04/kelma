//! The live call driver (feature `calls`): kelma's native calling engine behind the call window.
//!
//! A port of `kelma-cli call join` (`crates/kelma-cli/src/call.rs`), which was verified live:
//! discover, TURN, the calling trouter, a media session and the join; the lobby, whose admit
//! re-offer restarts ICE and is answered on a fresh media session (the sender follows a watch so
//! the audio pipeline survives the swap); keepAlive, the roster, the controls and leave.
//!
//! [`Hub`] owns the one calling trouter of the app (per endpoint id). It starts at sign-in, so
//! incoming call invitations always arrive, and routes every push: each live call's callbacks to
//! it (a call on hold keeps getting its own), invitations and every other push to the ringing
//! flows' subscriptions.
//!
//! Audio is the microphone through echo cancellation (AEC3, noise suppression, AGC2), fed by
//! what the speaker plays, as `call join --mic --speaker` runs it (`KELMA_NO_AEC=1` turns it off,
//! like `--no-aec`). `KELMA_CALL_DUMP=<dir>` writes every raw calling push there.

use super::bots::Bots;
use super::camera::{self, Camera, CameraMsg};
use super::remote_video::{self, RemoteVideo};
use super::share::{self, Share, ShareMsg};
use super::{
    CallDriver, CallHandle, Caption, Control, Device, Devices, Event, EventSink, JoinOptions,
    JoinTarget, Link, Person, Phase, live_phase, recording_state,
};
use anyhow::Context;
use calling::audio::{
    AudioProcessor, Capture, Playback, echo_cancelled, spawn_capture_encoder,
    spawn_decoder_playback,
};
use calling::captions::{CAPTIONS_DATA_ID, Depacketizer, parse_caption_message};
use calling::invitation::{CallEvent, NgcPush, classify};
use calling::media::{AudioSender, MediaEvents, MediaOptions, MediaSession, MediaState};
use calling::ngc_trouter::{CallPush, NgcEvent, NgcTrouter};
use calling::signaling::call_ops::{
    TransferOutcome, hold_offer, keep_media_running, parse_transfer_completion,
};
use calling::signaling::meeting::{MeetingTarget, meeting_from_csa_chat, meeting_from_join_url};
use calling::signaling::roster::{Presence, roster_snapshot};
use calling::signaling::{CallSession, Controls, NgcClient, Roster};
use calling::{CallInvitation, IceServer, Links, Modality, OpusPacket, SdpBlob};
use std::collections::HashMap;
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
    /// Where each live call's callbacks go, by its session id (the shown call and the one on
    /// hold).
    sessions: Mutex<HashMap<String, mpsc::UnboundedSender<CallPush>>>,
    invitations: Mutex<Vec<mpsc::UnboundedSender<Box<CallInvitation>>>>,
    unmatched: Mutex<Vec<mpsc::UnboundedSender<CallPush>>>,
}

impl Hub {
    pub fn new(client: Client) -> Arc<Hub> {
        Arc::new(Hub {
            client,
            trouter: Mutex::new(None),
            ngc: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
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
                let call = self.sessions.lock().unwrap().get(&session_id).cloned();
                match call {
                    Some(tx) => {
                        let _ = tx.send(p);
                    }
                    None => self.unmatched(p),
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

    /// Routes the callbacks of `session_id` to the returned receiver from now on, next to every
    /// other live call's.
    fn add_session(&self, session_id: &str) -> mpsc::UnboundedReceiver<CallPush> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.to_owned(), tx);
        rx
    }

    fn remove_session(&self, session_id: &str) {
        self.sessions.lock().unwrap().remove(session_id);
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
    // A meeting join also offers the gallery's receive-only lines (UNVERIFIED).
    let options = MediaOptions {
        receive_slots: remote_video::wanted_slots(),
        ..media_options(true)
    };
    let (media, events) = MediaSession::with_options(ice.clone(), options).await?;
    let offer = media.create_offer().await?;
    let me = hub.client.self_mri().await?;
    let s = CallSession::new(&surl, &me, &opts.name);
    // Callbacks can arrive before the join answers.
    let pushes = hub.add_session(s.callbacks.session_id());
    let conv = match ngc
        .join(&s, &target, &SdpBlob::ngc(offer), &MODALITIES)
        .await
    {
        Ok(c) => c,
        Err(e) => {
            hub.remove_session(s.callbacks.session_id());
            let _ = media.close().await;
            return Err(e.into());
        }
    };
    let mut roster = Roster::default();
    let mut bots = Bots::new(target.thread_id.clone(), true);
    if let Some(snapshot) = roster_snapshot(&conv.raw) {
        roster.apply_snapshot(snapshot);
        bots.roster(snapshot);
    }
    let mut answered = false;
    let mut video = RemoteVideo::default();
    if let Some(ans) = find_blob(&conv.raw) {
        match ans.decode() {
            Ok(sdp) => match media.set_answer(&sdp).await {
                Ok(()) => {
                    answered = true;
                    video.negotiated(&media, &sdp).await;
                }
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
        pending_offer: None,
        transferring: None,
        link: Link::New,
        share: Share::default(),
        bots,
        camera: Camera::default(),
        video,
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
    let pushes = hub.add_session(session.callbacks.session_id());
    tokio::spawn(async move {
        let ngc = match hub.ngc_client().await {
            Ok(n) => n,
            Err(e) => {
                hub.remove_session(session.callbacks.session_id());
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
            pending_offer: None,
            transferring: None,
            link: Link::Connected,
            share: Share::default(),
            bots: Bots::new(None, false),
            camera: Camera::default(),
            video: RemoteVideo::default(),
        };
        // A direct call's sharing line shows the other side's screen (no subscription).
        call.video.negotiated(&call.media, "").await;
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
/// `call join --no-aec`, for comparing both live).
fn aec_wanted() -> bool {
    aec_wanted_by(std::env::var_os("KELMA_NO_AEC").as_deref())
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
/// With `aec`, the speaker must report each played frame to the returned reference (its delay
/// to the microphone is estimated inside from the devices' measured latency).
fn mic_chain<S: calling::audio::FrameSource + 'static>(
    capture: S,
    aec: bool,
) -> (
    mpsc::Receiver<OpusPacket>,
    Option<calling::audio::EchoReference>,
) {
    if aec {
        let (mic, reference) = echo_cancelled(capture, AudioProcessor::new());
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
    /// On hold: nothing is sent or played.
    held: Arc<AtomicBool>,
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
        let (mut encoded, reference) = mic_chain(capture, aec_wanted());
        let out = match reference {
            Some(r) => Playback::start_with_tap(speaker, move |f| r.push(f)),
            None => Playback::start(speaker),
        }
        .context("speaker")?;
        self.play_into(out);
        let (sender, muted, held) = (self.sender.clone(), self.muted.clone(), self.held.clone());
        self.send_task = Some(tokio::spawn(async move {
            while let Some(p) = encoded.recv().await {
                // Muting and hold are also local: the packets stop here.
                if muted.load(Ordering::Relaxed) || held.load(Ordering::Relaxed) {
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

/// Moves one media session's incoming packets to whatever playback is current (none on hold).
fn forward(
    play: watch::Receiver<Option<mpsc::Sender<OpusPacket>>>,
    held: Arc<AtomicBool>,
    mut incoming: mpsc::Receiver<OpusPacket>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(p) = incoming.recv().await {
            if held.load(Ordering::Relaxed) {
                continue;
            }
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

/// A call's media options: [`share::media_options`], plus in a meeting the data channel that
/// carries live captions.
fn media_options(meeting: bool) -> MediaOptions {
    MediaOptions {
        data_channel: meeting,
        ..share::media_options()
    }
}

/// The captions in one data-channel frame, once the message it ends is whole. A caption without
/// the speaker's name takes it from the roster by the speaker's id.
fn captions_in(dp: &mut Depacketizer, frame: &[u8], roster: &Roster) -> Vec<Caption> {
    let Some(msg) = dp.push(frame) else {
        return Vec::new();
    };
    if msg.data_id != CAPTIONS_DATA_ID {
        return Vec::new();
    }
    parse_caption_message(&msg.payload)
        .into_iter()
        .map(|l| Caption {
            speaker: if l.speaker.is_empty() {
                speaker_name(roster, &l.user_id)
            } else {
                l.speaker
            },
            id: l.id,
            text: l.text,
        })
        .collect()
}

/// The roster's name for a caption's `userId`: the MRI itself, or (UNVERIFIED which one the
/// service sends) the id after the MRI's prefix.
fn speaker_name(roster: &Roster, user_id: &str) -> String {
    if user_id.is_empty() {
        return String::new();
    }
    roster
        .get(user_id)
        .or_else(|| {
            roster.participants().iter().find(|p| {
                p.mri
                    .rsplit_once(':')
                    .is_some_and(|(_, id)| id.eq_ignore_ascii_case(user_id))
            })
        })
        .map(|p| p.display_name.clone())
        .unwrap_or_default()
}

fn people(roster: &Roster, me: &str, talk: &Talk, video: &RemoteVideo) -> Vec<Person> {
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
            video_slot: video.slot_of(&p.mri),
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
    share: Share,
    /// Captions, recording and transcription (the recorder bot).
    bots: Bots,
    /// Our re-offer (hold, resume) as the media session made it, while its answer is due on the
    /// `call/mediaAnswer` callback.
    pending_offer: Option<String>,
    /// Who we are transferring the call to, until `call/transferCompletion`.
    transferring: Option<String>,
    camera: Camera,
    /// Other people's video: the 1:1 camera, a shared screen, the meeting gallery.
    video: RemoteVideo,
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

    /// Applies the media answer once; true when it was applied now.
    async fn apply_answer(&mut self, answer: &SdpBlob) -> bool {
        if self.answered {
            return false;
        }
        match answer.decode() {
            Ok(sdp) => match self.media.set_answer(&sdp).await {
                Ok(()) => {
                    self.answered = true;
                    self.video.negotiated(&self.media, &sdp).await;
                    return true;
                }
                Err(e) => tracing::warn!(%e, "media answer refused"),
            },
            Err(e) => tracing::warn!(%e, "undecodable media answer"),
        }
        false
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
            // Share and Camera are handled by the run loop, which owns their channels.
            Control::Share(_) | Control::Camera(_) | Control::Leave => {}
            Control::Captions(_) | Control::Recording(_) | Control::Transcription(_) => {
                self.bots.ask(&c);
                self.step_bots(sink).await;
            }
            Control::Hold(on) => self.hold(on, audio, sink).await,
            Control::AddPerson { mri, name } => {
                if let Err(e) = self
                    .ngc
                    .add_participant(s, links, controller, &mri, &name)
                    .await
                {
                    tracing::warn!(%e, "add person failed");
                    sink.send(Event::Error(format!("Couldn't add {name} to the call.")));
                }
            }
            Control::Transfer { mri, name } => match self.ngc.transfer(s, links, &mri).await {
                Ok(()) => self.transferring = Some(name),
                Err(e) => {
                    tracing::warn!(%e, "transfer failed");
                    sink.send(Event::Error(format!(
                        "Couldn't transfer the call to {name}."
                    )));
                }
            },
        }
    }

    /// Lets the recorder bot's side send what it can now.
    async fn step_bots(&mut self, sink: &EventSink) {
        let (s, links, controller) = (&self.s, &self.links, self.controller.as_str());
        self.bots.step(&self.ngc, s, links, controller, sink).await;
    }

    /// Hold (`on`) or resume: silence our side at once, then re-offer the media on the same
    /// peer connection, every line `inactive` to hold (see [`calling::signaling::call_ops`]).
    /// A refused re-offer puts the hold back as it was.
    async fn hold(&mut self, on: bool, audio: &mut Audio, sink: &EventSink) {
        audio.held.store(on, Ordering::Relaxed);
        let sent = async {
            let ours = self.media.create_offer().await?;
            let signaled = if on { hold_offer(&ours) } else { ours.clone() };
            let inline = self
                .ngc
                .renegotiate(&self.s, &self.links, &MODALITIES, &SdpBlob::ngc(signaled))
                .await?;
            anyhow::Ok((ours, inline))
        }
        .await;
        match sent {
            Ok((ours, Some(answer))) => {
                self.apply_reoffer_answer(&answer, &ours, &Links::default(), sink)
                    .await
            }
            Ok((ours, None)) => self.pending_offer = Some(ours),
            Err(e) => {
                tracing::warn!("hold {on} failed: {e:#}");
                audio.held.store(!on, Ordering::Relaxed);
                sink.send(Event::Held(!on));
                sink.send(Event::Error(if on {
                    "Couldn't put the call on hold.".into()
                } else {
                    "Couldn't resume the call.".into()
                }));
            }
        }
    }

    /// Applies the answer to our hold or resume re-offer `ours`, with our directions mirrored
    /// back so the media keeps running locally, and acknowledges it.
    async fn apply_reoffer_answer(
        &mut self,
        answer: &SdpBlob,
        ours: &str,
        links: &Links,
        sink: &EventSink,
    ) {
        let applied = match answer.decode() {
            Ok(sdp) => self
                .media
                .set_answer(&keep_media_running(&sdp, ours))
                .await
                .map_err(anyhow::Error::from),
            Err(e) => Err(anyhow::Error::from(e)),
        };
        if let Err(e) = applied {
            tracing::warn!("hold answer not applied: {e:#}");
            sink.send(Event::Error("The call's media update was refused.".into()));
            return;
        }
        if let Some(url) = links.get("mediaAcknowledgement")
            && let Err(e) = self.ngc.acknowledge_media(&self.s, url).await
        {
            tracing::warn!(%e, "media answer not acknowledged");
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
            held: Arc::new(AtomicBool::new(false)),
            play: play_tx,
            send_task: None,
            playback: None,
        };
        if let Err(e) = audio.start(opts.mic.as_deref(), opts.speaker.as_deref()) {
            sink.send(Event::Error(format!("Audio device: {e:#}")));
        }
        let mut forwarders = vec![forward(play_rx.clone(), audio.held.clone(), events.audio)];
        let mut state_rx = events.state;
        let mut speakers_rx = events.speakers;
        let mut data_rx = events.data;
        let mut depacketizer = Depacketizer::default();
        self.video.show(events.video, &sink);
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
        // A finished transfer ends the call, and we still leave our leg.
        let mut hang_up = false;
        let (share_tx, mut share_rx) = mpsc::unbounded_channel();
        let (camera_tx, mut camera_rx) = mpsc::unbounded_channel();
        // Joined with the camera on: it starts once the call can send it.
        let mut camera_wanted = opts.camera;
        let ended: Option<String> = loop {
            let mut roster_changed = false;
            tokio::select! {
                c = ctl.recv() => match c {
                    None | Some(Control::Leave) => break None,
                    Some(Control::Share(on)) => self.share_control(on, &share_tx, &sink).await,
                    Some(Control::Camera(on)) => self.camera_control(on, &camera_tx, &sink).await,
                    Some(c) => self.control(c, &mut audio, &sink).await,
                },
                Some(m) = share_rx.recv() => self.share_msg(m, &share_tx, &sink).await,
                Some(frame) = data_rx.recv() => {
                    for c in captions_in(&mut depacketizer, &frame, &self.roster) {
                        sink.send(Event::Caption(c));
                    }
                }
                Some(m) = camera_rx.recv() => self.camera_msg(m, &camera_tx, &sink).await,
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
                                // New lines: the subscriptions follow.
                                roster_changed |= self.apply_answer(&a).await;
                            }
                            self.start_keep_alive(keep_alive_interval_s);
                        }
                        CallEvent::MediaAnswer { answer, links } => {
                            match self.pending_offer.take() {
                                Some(ours) => self.apply_reoffer_answer(&answer, &ours, &links, &sink).await,
                                None => roster_changed |= self.apply_answer(&answer).await,
                            }
                            self.links.0.extend(links.0);
                        }
                        CallEvent::MediaRenegotiation { offer, links } => {
                            let swapped = self.renegotiate(&offer, &links, &sender_tx, &sink).await;
                            if let Some(ev) = swapped {
                                // The share was sent on the old session's line.
                                if self.share.stop().await {
                                    sink.send(Event::Error("Screen sharing stopped: the call's media restarted.".into()));
                                }
                                // So was the camera: it comes back on the new session's line.
                                camera_wanted |= self.camera.stop(&sink).await;
                                self.video.show(ev.video, &sink);
                                state_rx = ev.state;
                                speakers_rx = ev.speakers;
                                speakers_open = true;
                                data_rx = ev.data;
                                depacketizer = Depacketizer::default();
                                forwarders.push(forward(play_rx.clone(), audio.held.clone(), ev.audio));
                            }
                            self.links.0.extend(links.0);
                            // Admission re-offers the media; we are out of the lobby.
                            in_lobby = false;
                            // The lines may have changed: the subscriptions start over.
                            roster_changed = true;
                        }
                        CallEvent::Rejected(r) | CallEvent::Ended(r) => {
                            tracing::info!(code = r.code, sub = r.sub_code, "call ended by the service");
                            break Some(self.ended_text());
                        }
                        CallEvent::RosterUpdate(delta) => {
                            roster_changed = self.roster.apply(&delta);
                            self.bots.roster(&delta);
                            self.step_bots(&sink).await;
                        }
                        CallEvent::ConversationUpdate(body) => {
                            if let Some(snapshot) = roster_snapshot(&body) {
                                roster_changed = self.roster.apply_snapshot(snapshot);
                                self.bots.roster(snapshot);
                            }
                            if let Some(on) = recording_state(&body) {
                                sink.send(Event::Recording(on));
                            }
                            for ev in self.bots.conversation(&body) {
                                sink.send(ev);
                            }
                            self.step_bots(&sink).await;
                        }
                        CallEvent::Other { event, body, .. } if event == "addParticipantFailure" => {
                            // The recorder bot's invitation when one is pending, else a person
                            // from the invite box.
                            let refused = self.bots.bot_refused(&body);
                            if refused.is_empty() {
                                tracing::warn!("adding a person failed");
                                sink.send(Event::Error("Couldn't add that person to the call.".into()));
                            }
                            for ev in refused {
                                sink.send(ev);
                            }
                        }
                        CallEvent::Other { scope, event, body }
                            if scope == "call" && event == "transferCompletion" =>
                        {
                            match parse_transfer_completion(&body) {
                                Some(TransferOutcome::Done) => {
                                    let to = self.transferring.take().unwrap_or_default();
                                    hang_up = true;
                                    break Some(format!("You transferred the call to {to}."));
                                }
                                Some(TransferOutcome::Failed { code, sub_code }) => {
                                    tracing::warn!(code, sub_code, "transfer failed");
                                    self.transferring = None;
                                    sink.send(Event::Error("The transfer didn't go through. The call is still on hold.".into()));
                                }
                                None => {}
                            }
                        }
                        CallEvent::MediaAcknowledgement { links } => self.links.0.extend(links.0),
                        CallEvent::Other { event, body, .. } if event == "controlVideoStreaming" => {
                            self.share.sender_control(&body);
                            self.camera.sender_control(&body);
                        }
                        CallEvent::Progress { .. } | CallEvent::Other { .. } => {}
                    }
                }
            }
            if roster_changed {
                if self.meeting {
                    let spoke = |m: &str| talk.last.get(m).copied().unwrap_or(0);
                    self.video.reselect(
                        &self.ngc,
                        &self.s,
                        &self.links,
                        &self.roster,
                        spoke,
                        &sink,
                    );
                }
                if let Some(name) = self.video.presenter_changed(&self.roster, &me) {
                    sink.send(Event::Presenter(name));
                }
                sink.send(Event::Roster(people(&self.roster, &me, &talk, &self.video)));
                if let Some(p) = self.roster.get(&me) {
                    in_lobby = p.presence == Presence::Lobby;
                    let hand = p.hand.is_some();
                    if own_hand.replace(hand).is_some_and(|h| h != hand) {
                        sink.send(Event::Hand(hand));
                    }
                }
            }
            let can_share = self.link == Link::Connected
                && share::available(&self.media, &self.links, self.meeting);
            if let Some((available, sharing)) = self.share.changed(can_share) {
                sink.send(Event::Share { available, sharing });
            }
            let can_camera = self.link == Link::Connected
                && camera::available(&self.media, &self.links, self.meeting);
            if can_camera && std::mem::take(&mut camera_wanted) {
                self.camera.open(&camera_tx);
            }
            if let Some((available, on)) = self.camera.changed(can_camera) {
                sink.send(Event::Camera { available, on });
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

        if ended.is_none() || hang_up {
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
        self.share.stop().await;
        self.camera.stop(&sink).await;
        self.video.stop(&sink);
        audio.stop();
        let _ = self.media.close().await;
        for f in forwarders {
            f.abort();
        }
        self.hub.remove_session(self.s.callbacks.session_id());
        if let Some(reason) = ended {
            sink.send(Event::Ended(reason));
        }
    }

    /// Share / Stop sharing: the picker opens without blocking the call (its answer comes as a
    /// [`ShareMsg`]); stopping tells the service at once.
    async fn share_control(
        &mut self,
        on: bool,
        tx: &mpsc::UnboundedSender<ShareMsg>,
        sink: &EventSink,
    ) {
        if on {
            self.share.pick(tx);
        } else if self.share.stop().await {
            self.tell_share(false, sink).await;
        }
    }

    async fn share_msg(
        &mut self,
        m: ShareMsg,
        tx: &mpsc::UnboundedSender<ShareMsg>,
        sink: &EventSink,
    ) {
        match m {
            ShareMsg::Picked(Ok(Some(capture))) => {
                match self.share.start(*capture, &self.media, tx) {
                    Ok(()) => self.tell_share(true, sink).await,
                    Err(e) => {
                        tracing::warn!(%e, "screen share not started");
                        self.share.stop().await;
                        sink.send(Event::Error(format!("Couldn't share your screen: {e}")));
                    }
                }
            }
            ShareMsg::Picked(Ok(None)) => self.share.picked_nothing(),
            ShareMsg::Picked(Err(e)) => {
                self.share.picked_nothing();
                tracing::warn!(%e, "screen capture failed");
                sink.send(Event::Error(format!("Couldn't share your screen: {e}")));
            }
            ShareMsg::Ended => {
                if self.share.stop().await {
                    self.tell_share(false, sink).await;
                }
            }
        }
    }

    /// Tells the service our sharing line sends (or stopped): `UpdateMediaDescriptions` in a
    /// meeting, a re-offer in a 1:1 call.
    async fn tell_share(&mut self, sending: bool, sink: &EventSink) {
        let r = if self.meeting {
            share::tell_meeting(
                &self.ngc,
                &self.s,
                &self.links,
                &self.media,
                &mut self.share,
                sending,
            )
            .await
        } else {
            // The answer comes inline or on `call/mediaAnswer/`, which `apply_answer` takes.
            self.answered = false;
            match share::reoffer_direct(&self.ngc, &self.s, &self.links, &self.media, sending).await
            {
                Ok(Some(a)) => {
                    self.apply_answer(&a).await;
                    Ok(())
                }
                Ok(None) => Ok(()),
                Err(e) => {
                    self.answered = true;
                    Err(e)
                }
            }
        };
        if let Err(e) = r {
            tracing::warn!("share not signalled: {e:#}");
            if sending {
                self.share.stop().await;
                sink.send(Event::Error(
                    "Others can't see your screen: the call refused the share.".into(),
                ));
            }
        }
    }

    /// Camera on / off: the portal opens without blocking the call (its answer comes as a
    /// [`CameraMsg`]); turning it off tells the service at once.
    async fn camera_control(
        &mut self,
        on: bool,
        tx: &mpsc::UnboundedSender<CameraMsg>,
        sink: &EventSink,
    ) {
        if on {
            self.camera.open(tx);
        } else if self.camera.stop(sink).await {
            self.tell_camera(false, sink).await;
        }
    }

    async fn camera_msg(
        &mut self,
        m: CameraMsg,
        tx: &mpsc::UnboundedSender<CameraMsg>,
        sink: &EventSink,
    ) {
        match m {
            CameraMsg::Opened(Ok(capture)) => {
                match self.camera.start(*capture, &self.media, tx, sink) {
                    Ok(()) => self.tell_camera(true, sink).await,
                    Err(e) => {
                        tracing::warn!(%e, "camera not started");
                        self.camera.stop(sink).await;
                        sink.send(Event::Error(format!("Couldn't turn on your camera: {e}")));
                    }
                }
            }
            CameraMsg::Opened(Err(e)) => {
                self.camera.open_failed();
                tracing::warn!(%e, "camera not opened");
                sink.send(Event::Error(format!("Couldn't turn on your camera: {e}")));
            }
            CameraMsg::Ended => {
                if self.camera.stop(sink).await {
                    self.tell_camera(false, sink).await;
                }
            }
        }
    }

    /// Tells the service our camera sends (or stopped): `UpdateMediaDescriptions` in a meeting,
    /// a re-offer in a 1:1 call.
    async fn tell_camera(&mut self, on: bool, sink: &EventSink) {
        let r = if self.meeting {
            camera::tell_meeting(
                &self.ngc,
                &self.s,
                &self.links,
                &self.media,
                &mut self.camera,
                on,
            )
            .await
        } else {
            // The answer comes inline or on `call/mediaAnswer/`, which `apply_answer` takes.
            self.answered = false;
            let sharing = self.share.sharing();
            match camera::reoffer_direct(&self.ngc, &self.s, &self.links, &self.media, on, sharing)
                .await
            {
                Ok(Some(a)) => {
                    self.apply_answer(&a).await;
                    Ok(())
                }
                Ok(None) => Ok(()),
                Err(e) => {
                    self.answered = true;
                    Err(e)
                }
            }
        };
        if let Err(e) = r {
            tracing::warn!("camera not signalled: {e:#}");
            if on {
                self.camera.stop(sink).await;
                sink.send(Event::Error(
                    "Others can't see your video: the call refused it.".into(),
                ));
            }
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
                match MediaSession::with_options(self.ice.clone(), media_options(self.meeting))
                    .await
                {
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
                // The media server's offer names its lines' stream ids.
                self.video.negotiated(&self.media, &sdp).await;
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

    #[tokio::test]
    async fn the_mic_chain_runs_through_the_echo_canceller() {
        use calling::audio::{EchoReference, ToneSource};
        // With AEC the speaker gets a reference to feed; it accepts a played 20 ms frame.
        let (mut packets, reference) = mic_chain(ToneSource::endless(440.0), true);
        let reference: EchoReference = reference.expect("AEC stage in the chain");
        reference.push(&[0i16; calling::FRAME_SAMPLES]);
        assert!(packets.recv().await.is_some(), "the chain still encodes");
        // The opt-out has no AEC stage and nothing to feed.
        let (mut raw, none) = mic_chain(ToneSource::endless(440.0), false);
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
        let p = people(&r, "8:orgid:me", &Talk::default(), &RemoteVideo::default());
        let me = p.iter().find(|p| p.me).unwrap();
        assert!(me.muted && !me.lobby);
        let guest = p.iter().find(|p| !p.me).unwrap();
        assert_eq!((guest.name.as_str(), guest.lobby), ("Unknown", true));
    }

    #[test]
    fn pushes_route_to_every_live_call() {
        let hub = Hub::new(Client::offline("t").unwrap());
        let mut unmatched = hub.subscribe_unmatched();
        let mut shown = hub.add_session("sess-1");
        let mut held = hub.add_session("sess-2");
        let push = |session: &str| CallPush {
            id: 1,
            url: format!("https://surl/callAgent/{session}/abcd1234/conversation/rosterUpdate/"),
            headers: Default::default(),
            body: json!({"participants": {}}),
        };
        hub.route(push("sess-1"));
        hub.route(push("sess-2"));
        hub.route(push("sess-3"));
        assert!(shown.try_recv().unwrap().url.contains("sess-1"));
        assert!(shown.try_recv().is_err());
        assert!(
            held.try_recv().unwrap().url.contains("sess-2"),
            "the held call's own"
        );
        assert!(unmatched.try_recv().unwrap().url.contains("sess-3"));
        // One call ending leaves the other routed.
        hub.remove_session("sess-1");
        hub.route(push("sess-1"));
        hub.route(push("sess-2"));
        assert!(unmatched.try_recv().unwrap().url.contains("sess-1"));
        assert!(held.try_recv().is_ok());
    }

    #[test]
    fn pushes_route_to_the_active_call_or_the_subscribers() {
        let hub = Hub::new(Client::offline("t").unwrap());
        let mut unmatched = hub.subscribe_unmatched();
        let mut active = hub.add_session("sess-1");
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
        hub.remove_session("sess-1");
        hub.route(push("sess-1"));
        assert!(unmatched.try_recv().is_ok(), "no active call: unmatched");
    }
}
