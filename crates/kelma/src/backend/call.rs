//! The active call: one meeting or 1:1 call at a time, behind the call window (plans/meetings.md
//! M5.3, plans/calendar.md C3.2).
//!
//! [`CallManager`] owns the call's state as a small state machine ([`CallModel`]) and pushes it to
//! the `CallStore` global after every change. The network and audio side is a [`CallDriver`]:
//! - the live one (`call/live.rs`, feature `calls`) runs the native calling engine of
//!   `crates/calling`: discover, TURN, the calling trouter, the media session and join, the
//!   lobby's admit re-offer, keepAlive, the roster, the controls and leave;
//! - [`FakeDriver`] scripts the same events with no network or audio device, for the UI tests and
//!   the demo (`KELMA_DEMO`, `kelma --call-demo`).
//!
//! A driver reports through an [`EventSink`]; the manager sends it [`Control`]s through the
//! [`CallHandle`] it returned. The UI sends `CallStore.command(name, argument)`, which
//! [`CallCmd::parse`] reads.

use super::Engine;
use crate::data::Pixels;
use crate::ui::UiHandle;
#[cfg(any(test, feature = "calls"))]
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

#[cfg(feature = "calls")]
mod bots;
#[cfg(feature = "calls")]
pub(crate) mod live;

/// What kind of call the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallKind {
    Meeting {
        title: String,
    },
    /// A 1:1 call handed over by `call_direct` ([`CallManager::adopt`]).
    #[cfg(any(test, feature = "calls"))]
    Direct {
        peer_name: String,
        /// The 1:1 chat with the peer (the call window's Chat opens it), when known.
        chat_id: String,
    },
}

impl Default for CallKind {
    fn default() -> Self {
        CallKind::Meeting {
            title: String::new(),
        }
    }
}

impl CallKind {
    pub fn title(&self) -> &str {
        match self {
            CallKind::Meeting { title } => title,
            #[cfg(any(test, feature = "calls"))]
            CallKind::Direct { peer_name, .. } => peer_name,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            CallKind::Meeting { .. } => "meeting",
            #[cfg(any(test, feature = "calls"))]
            CallKind::Direct { .. } => "direct",
        }
    }
}

/// Where the call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// No call, no window.
    #[default]
    Idle,
    /// The pre-join screen.
    PreJoin,
    Connecting,
    /// Waiting for someone in the meeting to admit us.
    Lobby,
    Connected,
    /// The media connection dropped and may come back (the live driver reports it).
    #[cfg(any(test, feature = "calls"))]
    Reconnecting,
    /// The call-ended screen.
    Ended,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Idle => "",
            Phase::PreJoin => "prejoin",
            Phase::Connecting => "connecting",
            Phase::Lobby => "lobby",
            Phase::Connected => "connected",
            #[cfg(any(test, feature = "calls"))]
            Phase::Reconnecting => "reconnecting",
            Phase::Ended => "ended",
        }
    }

    /// A call is running (joined or joining).
    pub fn active(self) -> bool {
        match self {
            Phase::Connecting | Phase::Lobby | Phase::Connected => true,
            #[cfg(any(test, feature = "calls"))]
            Phase::Reconnecting => true,
            Phase::Idle | Phase::PreJoin | Phase::Ended => false,
        }
    }
}

/// The media connection as the live driver sees it (`MediaState` without the calling types).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(any(test, feature = "calls"))]
pub enum Link {
    New,
    Connecting,
    Connected,
    Disconnected,
}

/// The phase a live call is in. `in_lobby`: the roster shows us waiting in the lobby (the lobby
/// leg's media reaches Connected too, so the roster decides). `was_in_call`: the call was
/// connected outside the lobby before, so a drop is a reconnect.
#[cfg(any(test, feature = "calls"))]
pub fn live_phase(link: Link, in_lobby: bool, was_in_call: bool) -> Phase {
    match (in_lobby, link) {
        (true, _) => Phase::Lobby,
        (false, Link::Connected) => Phase::Connected,
        (false, _) if was_in_call => Phase::Reconnecting,
        (false, _) => Phase::Connecting,
    }
}

/// One person in the roster.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Person {
    /// Who they are, for their profile photo (empty when unknown).
    pub mri: String,
    pub name: String,
    pub muted: bool,
    pub hand: bool,
    /// Waiting in the lobby.
    pub lobby: bool,
    /// Us.
    pub me: bool,
    /// `organizer`, `presenter`, `attendee` or empty.
    pub role: String,
    /// Heard right now (the meeting mixer names their audio source).
    pub speaking: bool,
    /// When they last spoke, as a rising count (0: not yet); the floating call shows the latest
    /// speakers first.
    pub spoke: u64,
}

/// The profile photos `lookup` knows for these people, by MRI. A lookup may start a download, so
/// each MRI is asked once.
fn photos(people: &[Person], lookup: impl Fn(&str) -> Option<Pixels>) -> HashMap<String, Pixels> {
    let mut found = HashMap::new();
    let mut asked = HashSet::new();
    for p in people.iter().filter(|p| !p.mri.is_empty()) {
        if asked.insert(p.mri.as_str())
            && let Some(px) = lookup(&p.mri)
        {
            found.insert(p.mri.clone(), px);
        }
    }
    found
}

/// Finds a person's profile photo by MRI ([`CallManager::set_photos`]).
type PhotoLookup = Arc<dyn Fn(&str) -> Option<Pixels> + Send + Sync>;

/// One live caption: who speaks and what they said so far. A later caption with the same `id`
/// (the utterance) replaces it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caption {
    pub id: String,
    pub speaker: String,
    pub text: String,
}

/// How many caption lines the strip shows.
const CAPTION_LINES: usize = 2;

pub const RECORDING_NOTICE: &str =
    "Recording has started. By attending this meeting, you consent to being included.";
pub const TRANSCRIPTION_NOTICE: &str =
    "Transcription has started. By attending this meeting, you consent to being included.";

/// What a driver reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Connecting,
    Lobby,
    Connected,
    #[cfg(any(test, feature = "calls"))]
    Reconnecting,
    Roster(Vec<Person>),
    /// The meeting is (not) being recorded.
    Recording(bool),
    /// The meeting is (not) being transcribed.
    Transcribing(bool),
    /// Our live captions are on or off (off: the service refused them, or they stopped).
    Captions(bool),
    /// A live caption.
    Caption(Caption),
    /// Our hand (raised elsewhere, or lowered by an organizer).
    #[cfg(any(test, feature = "calls"))]
    Hand(bool),
    /// Whether the call is on hold (a hold or resume that failed puts it back).
    #[cfg(any(test, feature = "calls"))]
    Held(bool),
    /// Something went wrong; the call may go on.
    Error(String),
    /// The call is over, and why.
    Ended(String),
}

/// What the UI asks of a running call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Mute(bool),
    RaiseHand,
    LowerHand,
    /// Switch the microphone and speaker (device ids; `None` is the system default).
    Devices {
        mic: Option<String>,
        speaker: Option<String>,
    },
    /// Live captions on or off.
    Captions(bool),
    /// Start or stop recording the meeting.
    Recording(bool),
    /// Start or stop transcribing the meeting.
    Transcription(bool),
    /// Put the call on hold (`true`) or resume it.
    Hold(bool),
    /// Ring a person into the call (an MRI and their name).
    AddPerson {
        mri: String,
        name: String,
    },
    /// "Transfer now": hand the call over to a person and drop out.
    Transfer {
        mri: String,
        name: String,
    },
    Leave,
}

/// A person the call window's people search found (the invite box, the transfer dialog).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub mri: String,
    pub name: String,
    /// Job title or email.
    pub subtitle: String,
}

/// What to join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinTarget {
    /// A Teams join link (calendar event, join banner).
    Link(String),
    /// A meeting ID and passcode ("Join with an ID").
    Code { id: String, passcode: String },
    /// A meeting chat (`19:meeting_...@thread.v2`); the live driver looks its meeting up.
    Chat(String),
}

impl JoinTarget {
    /// Short, id-free text for logs and the fake's record.
    pub fn describe(&self) -> String {
        match self {
            JoinTarget::Link(_) => "link".into(),
            JoinTarget::Code { .. } => "code".into(),
            JoinTarget::Chat(id) => format!("chat:{id}"),
        }
    }
}

/// How to join, from the pre-join screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinOptions {
    pub name: String,
    pub mic_on: bool,
    /// Device ids for the live driver's audio (`None`: the system default).
    #[cfg(any(test, feature = "calls"))]
    pub mic: Option<String>,
    #[cfg(any(test, feature = "calls"))]
    pub speaker: Option<String>,
}

/// An audio device as the pickers list it. `id: None` is the system default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
}

/// Microphones and speakers, each with the one to use first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub mics: Vec<Device>,
    pub speakers: Vec<Device>,
    pub mic: usize,
    pub speaker: usize,
}

impl Devices {
    fn or_default(mut self) -> Self {
        for list in [&mut self.mics, &mut self.speakers] {
            if list.is_empty() {
                list.push(Device {
                    id: None,
                    name: "System default".into(),
                });
            }
        }
        self.mic = self.mic.min(self.mics.len() - 1);
        self.speaker = self.speaker.min(self.speakers.len() - 1);
        self
    }
}

/// The engine behind the call window.
pub trait CallDriver: Send + Sync {
    /// Native calls are available (else the calendar opens joins in the browser).
    fn native(&self) -> bool {
        true
    }
    fn devices(&self) -> Devices;
    /// Starts joining. Events go to `sink`; the handle takes the controls.
    fn join(&self, target: JoinTarget, opts: JoinOptions, sink: EventSink) -> Box<dyn CallHandle>;
}

/// A running call's control side. Dropping it is the same as [`Control::Leave`].
pub trait CallHandle: Send + Sync {
    fn control(&self, c: Control);
}

/// What the call window shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallData {
    pub phase: &'static str,
    pub kind: &'static str,
    pub title: String,
    pub my_name: String,
    pub timer: String,
    pub muted: bool,
    pub hand: bool,
    pub prejoin_mic: bool,
    pub recording: bool,
    pub transcribing: bool,
    pub captions: bool,
    pub caption_lines: Vec<Caption>,
    /// Teams' consent notice, shown when recording or transcription starts.
    pub notice: String,
    pub error: String,
    pub ended: String,
    pub chat_id: String,
    pub can_rejoin: bool,
    pub people: Vec<Person>,
    pub in_call: usize,
    pub in_lobby: usize,
    pub mics: Vec<String>,
    pub mic: usize,
    pub speakers: Vec<String>,
    pub speaker: usize,
    pub held: bool,
    pub found: Vec<Found>,
    /// A short status under the invite box ("Calling Ada…").
    pub invite_status: String,
}

/// `m:ss`, or `h:mm:ss` from an hour on.
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// The call's state, without the network: every transition is here, so it is unit tested.
#[derive(Debug, Clone, Default)]
pub struct CallModel {
    pub phase: Phase,
    pub kind: CallKind,
    pub target: Option<JoinTarget>,
    pub chat_id: String,
    pub my_name: String,
    /// The pre-join microphone choice.
    pub mic_on: bool,
    pub muted: bool,
    pub hand: bool,
    pub recording: bool,
    pub transcribing: bool,
    /// Live captions are on (asked for, and not refused).
    pub captions: bool,
    /// The latest captions, oldest first.
    pub caption_lines: Vec<Caption>,
    pub notice: String,
    pub roster: Vec<Person>,
    pub error: String,
    pub ended: String,
    pub connected_at: Option<Instant>,
    /// How long the call was connected, once it ended.
    pub duration: Option<Duration>,
    pub devices: Devices,
    /// On hold (by us).
    pub held: bool,
    /// The people search's results, for the latest query ([`Self::find_seq`]).
    pub found: Vec<Found>,
    pub find_seq: u64,
    /// A short status under the invite box ("Calling Ada…").
    pub invite_status: String,
}

pub const ALREADY_IN_CALL: &str = "You're already in a call. Leave it before joining another.";
pub const YOU_LEFT_MEETING: &str = "You left the meeting.";
#[cfg(any(test, feature = "calls"))]
pub const YOU_LEFT_CALL: &str = "You left the call.";

impl CallModel {
    /// Opens the pre-join screen. Refused while a call runs.
    pub fn prejoin(
        &mut self,
        kind: CallKind,
        target: JoinTarget,
        chat_id: String,
        devices: Devices,
    ) -> Result<(), &'static str> {
        if self.phase.active() {
            self.error = ALREADY_IN_CALL.into();
            return Err(ALREADY_IN_CALL);
        }
        *self = CallModel {
            phase: Phase::PreJoin,
            kind,
            target: Some(target),
            chat_id,
            my_name: std::mem::take(&mut self.my_name),
            mic_on: true,
            devices: devices.or_default(),
            ..Default::default()
        };
        Ok(())
    }

    /// Pre-join "Cancel".
    pub fn cancel(&mut self) -> bool {
        self.phase == Phase::PreJoin && {
            self.phase = Phase::Idle;
            true
        }
    }

    /// Pre-join "Join now": what to join and how. `None` unless on the pre-join screen.
    pub fn joining(&mut self) -> Option<(JoinTarget, JoinOptions)> {
        if self.phase != Phase::PreJoin {
            return None;
        }
        let target = self.target.clone()?;
        self.phase = Phase::Connecting;
        self.muted = !self.mic_on;
        Some((target, self.options()))
    }

    pub fn options(&self) -> JoinOptions {
        JoinOptions {
            name: self.my_name.clone(),
            mic_on: self.mic_on,
            #[cfg(any(test, feature = "calls"))]
            mic: self.mic_id(),
            #[cfg(any(test, feature = "calls"))]
            speaker: self.speaker_id(),
        }
    }

    fn mic_id(&self) -> Option<String> {
        self.devices.mics.get(self.devices.mic)?.id.clone()
    }

    fn speaker_id(&self) -> Option<String> {
        self.devices.speakers.get(self.devices.speaker)?.id.clone()
    }

    /// A connected call handed over (a 1:1 call answered or placed elsewhere).
    #[cfg(any(test, feature = "calls"))]
    pub fn adopted(&mut self, kind: CallKind, devices: Devices, now: Instant) -> bool {
        if self.phase.active() {
            return false;
        }
        let chat_id = match &kind {
            CallKind::Direct { chat_id, .. } => chat_id.clone(),
            CallKind::Meeting { .. } => String::new(),
        };
        *self = CallModel {
            phase: Phase::Connected,
            kind,
            chat_id,
            my_name: std::mem::take(&mut self.my_name),
            mic_on: true,
            connected_at: Some(now),
            devices: devices.or_default(),
            ..Default::default()
        };
        true
    }

    /// Applies a driver event. Events for no running call are ignored. Returns whether anything
    /// changed.
    pub fn apply(&mut self, ev: Event, now: Instant) -> bool {
        if !self.phase.active() {
            return false;
        }
        let next = match ev {
            Event::Connecting => {
                if !matches!(self.phase, Phase::Connecting | Phase::Lobby) {
                    return false;
                }
                Phase::Connecting
            }
            Event::Lobby => Phase::Lobby,
            Event::Connected => {
                self.connected_at.get_or_insert(now);
                Phase::Connected
            }
            #[cfg(any(test, feature = "calls"))]
            Event::Reconnecting => {
                if self.phase != Phase::Connected {
                    return false;
                }
                Phase::Reconnecting
            }
            Event::Roster(r) => return replace(&mut self.roster, r),
            Event::Recording(on) => {
                self.noticed(on && !self.recording, !on, RECORDING_NOTICE);
                return replace(&mut self.recording, on);
            }
            Event::Transcribing(on) => {
                self.noticed(on && !self.transcribing, !on, TRANSCRIPTION_NOTICE);
                return replace(&mut self.transcribing, on);
            }
            Event::Captions(on) => return self.set_captions(on),
            Event::Caption(c) => return self.caption(c),
            #[cfg(any(test, feature = "calls"))]
            Event::Hand(h) => return replace(&mut self.hand, h),
            #[cfg(any(test, feature = "calls"))]
            Event::Held(h) => return replace(&mut self.held, h),
            Event::Error(e) => return replace(&mut self.error, e),
            Event::Ended(reason) => {
                self.end(reason, now);
                return true;
            }
        };
        replace(&mut self.phase, next)
    }

    fn end(&mut self, reason: String, now: Instant) {
        self.phase = Phase::Ended;
        self.ended = reason;
        self.duration = self.connected_at.map(|t| now.saturating_duration_since(t));
        self.hand = false;
        self.recording = false;
        self.transcribing = false;
        self.notice.clear();
        self.set_captions(false);
        self.held = false;
        self.found.clear();
        self.invite_status.clear();
    }

    /// Shows `text` as the notice when `show`, clears it when `hide` and it is showing.
    fn noticed(&mut self, show: bool, hide: bool, text: &str) {
        if show {
            self.notice = text.into();
        } else if hide && self.notice == text {
            self.notice.clear();
        }
    }

    /// Captions on or off; off forgets the lines.
    pub fn set_captions(&mut self, on: bool) -> bool {
        if !on {
            self.caption_lines.clear();
        }
        replace(&mut self.captions, on)
    }

    /// A caption while captions are on: it updates its utterance, or comes last.
    fn caption(&mut self, c: Caption) -> bool {
        if !self.captions || c.text.trim().is_empty() {
            return false;
        }
        if let Some(known) = self.caption_lines.iter_mut().find(|l| l.id == c.id) {
            return replace(known, c);
        }
        self.caption_lines.push(c);
        if self.caption_lines.len() > CAPTION_LINES {
            self.caption_lines.remove(0);
        }
        true
    }

    /// "Leave": ends the call on our side at once. False when no call runs.
    pub fn leave(&mut self, now: Instant) -> bool {
        if !self.phase.active() {
            return false;
        }
        let reason = match self.kind {
            CallKind::Meeting { .. } => YOU_LEFT_MEETING,
            #[cfg(any(test, feature = "calls"))]
            CallKind::Direct { .. } => YOU_LEFT_CALL,
        };
        self.end(reason.into(), now);
        true
    }

    /// Closes the call-ended screen.
    pub fn close(&mut self) -> bool {
        self.phase == Phase::Ended && {
            self.phase = Phase::Idle;
            true
        }
    }

    /// "Rejoin" on the call-ended screen: back to the pre-join screen for the same meeting.
    pub fn rejoin(&mut self, devices: Devices) -> bool {
        match (self.phase, self.target.clone()) {
            (Phase::Ended, Some(t)) => {
                let (kind, chat) = (self.kind.clone(), self.chat_id.clone());
                self.prejoin(kind, t, chat, devices).is_ok()
            }
            _ => false,
        }
    }

    /// The mute toggle. Returns the new state while a call runs.
    pub fn toggle_mute(&mut self) -> Option<bool> {
        match self.phase {
            Phase::PreJoin => {
                self.mic_on = !self.mic_on;
                None
            }
            p if p.active() => {
                self.muted = !self.muted;
                Some(self.muted)
            }
            _ => None,
        }
    }

    /// The hand toggle. Returns the new state while a call runs.
    pub fn toggle_hand(&mut self) -> Option<bool> {
        self.phase.active().then(|| {
            self.hand = !self.hand;
            self.hand
        })
    }

    /// Hold / Resume. Returns the new state while the call is connected.
    pub fn toggle_hold(&mut self) -> Option<bool> {
        (self.phase == Phase::Connected).then(|| {
            self.held = !self.held;
            self.held
        })
    }

    /// Opening Transfer puts the call on hold first, as Teams does. Returns `Some(true)` when
    /// that changed the hold.
    pub fn transfer_opened(&mut self) -> Option<bool> {
        self.found.clear();
        if self.held {
            return None;
        }
        self.toggle_hold()
    }

    /// A new people search: its sequence number. Older answers are then dropped.
    pub fn find_started(&mut self) -> u64 {
        self.find_seq += 1;
        self.find_seq
    }

    /// The answer of search `seq`. False when it is stale.
    pub fn found(&mut self, seq: u64, found: Vec<Found>) -> bool {
        seq == self.find_seq && {
            self.found = found;
            true
        }
    }

    /// The person `mri` from the search, as a control `make` builds; the search is done.
    fn pick(&mut self, mri: &str, make: fn(String, String) -> Control) -> Option<Control> {
        if !self.phase.active() {
            return None;
        }
        let f = self.found.iter().find(|f| f.mri == mri)?.clone();
        self.found.clear();
        self.find_seq += 1;
        Some(make(f.mri, f.name))
    }

    /// The invite box's pick: ring that person into the call.
    pub fn invite(&mut self, mri: &str) -> Option<Control> {
        let c = self.pick(mri, |mri, name| Control::AddPerson { mri, name })?;
        if let Control::AddPerson { name, .. } = &c {
            self.invite_status = format!("Calling {name}…");
        }
        Some(c)
    }

    /// The transfer dialog's "Transfer now".
    pub fn transfer(&mut self, mri: &str) -> Option<Control> {
        self.pick(mri, |mri, name| Control::Transfer { mri, name })
    }

    /// Picks a microphone (`mic`) or speaker. Returns the device ids to switch to while a call
    /// runs.
    pub fn pick_device(&mut self, mic: bool, index: usize) -> Option<Control> {
        let (list, at) = if mic {
            (&self.devices.mics, &mut self.devices.mic)
        } else {
            (&self.devices.speakers, &mut self.devices.speaker)
        };
        if index >= list.len() || *at == index {
            return None;
        }
        *at = index;
        self.phase.active().then(|| Control::Devices {
            mic: self.mic_id(),
            speaker: self.speaker_id(),
        })
    }

    pub fn data(&self, now: Instant) -> CallData {
        let elapsed = match (self.phase, self.connected_at, self.duration) {
            (Phase::Ended, _, Some(d)) => d,
            (p, Some(t), _) if p.active() => now.saturating_duration_since(t),
            _ => Duration::ZERO,
        };
        let mut people = self.roster.clone();
        for p in people.iter_mut().filter(|p| p.me) {
            p.muted = self.muted;
            p.hand = self.hand;
        }
        // In the call first, the lobby after; each in roster order.
        people.sort_by_key(|p| p.lobby);
        let in_lobby = people.iter().filter(|p| p.lobby).count();
        CallData {
            phase: self.phase.name(),
            kind: self.kind.name(),
            title: self.kind.title().to_owned(),
            my_name: self.my_name.clone(),
            timer: clock(elapsed),
            muted: self.muted,
            hand: self.hand,
            prejoin_mic: self.mic_on,
            recording: self.recording,
            transcribing: self.transcribing,
            captions: self.captions,
            caption_lines: self.caption_lines.clone(),
            notice: self.notice.clone(),
            error: self.error.clone(),
            ended: self.ended.clone(),
            chat_id: self.chat_id.clone(),
            can_rejoin: self.target.is_some(),
            in_call: people.len() - in_lobby,
            in_lobby,
            people,
            mics: self.devices.mics.iter().map(|d| d.name.clone()).collect(),
            mic: self.devices.mic,
            speakers: self
                .devices
                .speakers
                .iter()
                .map(|d| d.name.clone())
                .collect(),
            speaker: self.devices.speaker,
            held: self.held,
            found: self.found.clone(),
            invite_status: self.invite_status.clone(),
        }
    }
}

fn replace<T: PartialEq>(slot: &mut T, v: T) -> bool {
    if *slot == v {
        return false;
    }
    *slot = v;
    true
}

/// Whether a conversation update says the meeting is recorded. UNVERIFIED: no capture shows the
/// field yet, so this reads the likely shapes (`recordingStatus`/`recordingState` set to
/// `Started`/`Active`/`Recording` or `Stopped`/`None`, a boolean `isRecording`) anywhere in the
/// body. `None` when the body says nothing about recording.
#[cfg(any(test, feature = "calls"))]
pub fn recording_state(body: &Value) -> Option<bool> {
    match body {
        Value::Object(m) => {
            for (k, v) in m {
                let key = k.to_ascii_lowercase();
                if key == "isrecording"
                    && let Some(b) = v.as_bool()
                {
                    return Some(b);
                }
                if (key == "recordingstatus" || key == "recordingstate")
                    && let Some(s) = v.as_str()
                {
                    return match s.to_ascii_lowercase().as_str() {
                        "started" | "active" | "recording" | "inprogress" => Some(true),
                        "stopped" | "none" | "ended" | "notstarted" => Some(false),
                        _ => None,
                    };
                }
            }
            m.values().find_map(recording_state)
        }
        Value::Array(a) => a.iter().find_map(recording_state),
        _ => None,
    }
}

/// A conversation that is a meeting's chat (the header's Join button shows on it).
pub fn is_meeting_chat(id: &str) -> bool {
    id.starts_with("19:meeting_")
}

/// What the call window sends as `CallStore.command(name, argument)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallCmd {
    /// The meeting chat header's Join: the pre-join screen for that chat's meeting.
    JoinChat(String),
    JoinNow,
    Cancel,
    /// The pre-join microphone switch.
    PrejoinMic(bool),
    Mic(usize),
    Speaker(usize),
    ToggleMute,
    ToggleHand,
    /// Hold / Resume.
    ToggleHold,
    /// The Transfer dialog opened (it holds the call).
    TransferOpen,
    /// Search people for the invite box or the transfer dialog.
    Find(String),
    /// The invite box picked a person (an MRI from the search).
    Invite(String),
    /// "Transfer now" to a person from the search.
    Transfer(String),
    Leave,
    /// Close the call-ended screen.
    Close,
    Rejoin,
    DismissError,
    /// More › Language and speech › Show live captions.
    Captions(bool),
    /// More › Record and transcribe › Start/Stop recording.
    Recording(bool),
    /// More › Record and transcribe › Start/Stop transcription.
    Transcription(bool),
    DismissNotice,
}

impl CallCmd {
    pub fn parse(name: &str, arg: &str) -> Option<CallCmd> {
        Some(match name {
            "join-chat" if is_meeting_chat(arg) => CallCmd::JoinChat(arg.into()),
            "join-now" => CallCmd::JoinNow,
            "cancel" => CallCmd::Cancel,
            "prejoin-mic" => CallCmd::PrejoinMic(arg == "on"),
            "mic" => CallCmd::Mic(arg.parse().ok()?),
            "speaker" => CallCmd::Speaker(arg.parse().ok()?),
            "mute" => CallCmd::ToggleMute,
            "hand" => CallCmd::ToggleHand,
            "hold" => CallCmd::ToggleHold,
            "transfer-open" => CallCmd::TransferOpen,
            "find" => CallCmd::Find(arg.into()),
            "invite" if !arg.is_empty() => CallCmd::Invite(arg.into()),
            "transfer" if !arg.is_empty() => CallCmd::Transfer(arg.into()),
            "leave" => CallCmd::Leave,
            "close" => CallCmd::Close,
            "rejoin" => CallCmd::Rejoin,
            "dismiss-error" => CallCmd::DismissError,
            "captions" => CallCmd::Captions(arg == "on"),
            "recording" => CallCmd::Recording(arg == "start"),
            "transcription" => CallCmd::Transcription(arg == "start"),
            "dismiss-notice" => CallCmd::DismissNotice,
            _ => return None,
        })
    }
}

/// Where a driver reports. Events from a call that is no longer the current one are dropped.
#[derive(Clone)]
pub struct EventSink {
    shared: Weak<Shared>,
    generation: u64,
}

impl EventSink {
    pub fn send(&self, ev: Event) {
        if let Some(s) = self.shared.upgrade() {
            s.apply(self.generation, ev);
        }
    }

    /// A sink attached to nothing (tests of a driver alone).
    #[cfg(test)]
    pub fn detached() -> Self {
        EventSink {
            shared: Weak::new(),
            generation: 0,
        }
    }
}

struct Shared {
    model: Mutex<CallModel>,
    handle: Mutex<Option<Arc<dyn CallHandle>>>,
    driver: Mutex<Arc<dyn CallDriver>>,
    generation: AtomicU64,
    ticking: AtomicBool,
    ui: UiHandle,
    photos: Mutex<Option<PhotoLookup>>,
}

impl Shared {
    fn push(&self) {
        let d = self.model.lock().unwrap().data(Instant::now());
        let lookup = self.photos.lock().unwrap().clone();
        let found = lookup
            .map(|f| photos(&d.people, |m| f(m)))
            .unwrap_or_default();
        self.ui.call_state(d, found);
    }

    fn apply(self: &Arc<Self>, generation: u64, ev: Event) {
        if generation != self.generation.load(Ordering::SeqCst) {
            return;
        }
        let (changed, phase) = {
            let mut m = self.model.lock().unwrap();
            (m.apply(ev, Instant::now()), m.phase)
        };
        if phase == Phase::Ended {
            // The driver is done; dropping the handle stops whatever it still runs.
            self.handle.lock().unwrap().take();
        }
        self.publish_presence();
        if changed {
            self.push();
        }
        if phase == Phase::Connected {
            self.tick();
        }
    }

    /// "In a call" while connected (or reconnecting), Available once the call is over.
    fn publish_presence(&self) {
        let (phase, meeting) = {
            let m = self.model.lock().unwrap();
            (m.phase, matches!(m.kind, CallKind::Meeting { .. }))
        };
        // Connected or reconnecting: active, and past joining.
        let connected =
            phase.active() && !matches!(phase, Phase::PreJoin | Phase::Connecting | Phase::Lobby);
        super::call_presence::update(connected, meeting);
    }

    /// Pushes the timer once a second while the call runs.
    fn tick(self: &Arc<Self>) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.ticking.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(self);
        rt.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(s) = weak.upgrade() else { return };
                if !s.model.lock().unwrap().phase.active() {
                    s.ticking.store(false, Ordering::SeqCst);
                    return;
                }
                s.push();
            }
        });
    }

    fn control(&self, c: Control) {
        // Out of the lock: a driver may report (and end the call) from inside `control`.
        let h = self.handle.lock().unwrap().clone();
        if let Some(h) = h {
            h.control(c);
        }
    }
}

/// Holds at most one active call and drives the call window.
#[derive(Clone)]
pub struct CallManager {
    shared: Arc<Shared>,
    #[cfg(feature = "calls")]
    hub: Arc<live::Hub>,
}

/// Used when kelma is built without the `calls` feature: nothing joins natively.
#[cfg(any(test, not(feature = "calls")))]
struct Unavailable;

#[cfg(any(test, not(feature = "calls")))]
impl CallDriver for Unavailable {
    fn native(&self) -> bool {
        false
    }

    fn devices(&self) -> Devices {
        Devices::default()
    }

    fn join(&self, _: JoinTarget, _: JoinOptions, sink: EventSink) -> Box<dyn CallHandle> {
        sink.send(Event::Error(
            "Calling isn't built into this version of kelma.".into(),
        ));
        sink.send(Event::Ended("Couldn't join.".into()));
        Box::new(NoHandle)
    }
}

#[cfg(any(test, not(feature = "calls")))]
struct NoHandle;

#[cfg(any(test, not(feature = "calls")))]
impl CallHandle for NoHandle {
    fn control(&self, _: Control) {}
}

impl CallManager {
    /// The manager with the live driver (or [`Unavailable`] without the `calls` feature).
    pub fn new(ui: UiHandle, client: teams_proto::Client, my_name: String) -> Self {
        #[cfg(not(test))]
        super::call_presence::install(client.clone());
        #[cfg(feature = "calls")]
        let hub = live::Hub::new(client);
        // Tests never reach the network or the sound cards.
        #[cfg(test)]
        let driver: Arc<dyn CallDriver> = Arc::new(FakeDriver::manual());
        #[cfg(all(feature = "calls", not(test)))]
        let driver: Arc<dyn CallDriver> = Arc::new(live::LiveDriver::new(hub.clone()));
        #[cfg(all(not(feature = "calls"), not(test)))]
        let driver: Arc<dyn CallDriver> = Arc::new(Unavailable);
        #[cfg(not(feature = "calls"))]
        let _ = client;
        CallManager {
            shared: Arc::new(Shared {
                model: Mutex::new(CallModel {
                    my_name,
                    ..Default::default()
                }),
                handle: Mutex::new(None),
                driver: Mutex::new(driver),
                generation: AtomicU64::new(0),
                ticking: AtomicBool::new(false),
                ui,
                photos: Mutex::new(None),
            }),
            #[cfg(feature = "calls")]
            hub,
        }
    }

    /// Starts the calling signalling at sign-in (the calling trouter, so incoming calls arrive).
    pub fn start(&self) {
        #[cfg(feature = "calls")]
        {
            self.hub.start();
            *MANAGER.lock().unwrap() = Some(Arc::new(self.clone()));
        }
    }

    /// Where the call window's profile photos come from. The call state is pushed every second
    /// while connected, so a photo still downloading shows up on a later push.
    pub fn set_photos(&self, lookup: impl Fn(&str) -> Option<Pixels> + Send + Sync + 'static) {
        *self.shared.photos.lock().unwrap() = Some(Arc::new(lookup));
    }

    /// Replaces the driver (tests and the demo use [`FakeDriver`]).
    pub fn set_driver(&self, d: Arc<dyn CallDriver>) {
        *self.shared.driver.lock().unwrap() = d;
    }

    fn driver(&self) -> Arc<dyn CallDriver> {
        self.shared.driver.lock().unwrap().clone()
    }

    /// Native calls are available; when not, joins open in the browser.
    pub fn native(&self) -> bool {
        self.driver().native()
    }

    /// Opens the pre-join screen for a meeting. A running call stays; the window shows why.
    pub fn prejoin(&self, title: String, target: JoinTarget, chat_id: String) {
        let devices = self.driver().devices();
        let _ = self.shared.model.lock().unwrap().prejoin(
            CallKind::Meeting { title },
            target,
            chat_id,
            devices,
        );
        self.shared.push();
    }

    /// Pre-join "Join now".
    pub fn join_now(&self) {
        let Some((target, opts)) = self.shared.model.lock().unwrap().joining() else {
            return;
        };
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.push();
        let sink = EventSink {
            shared: Arc::downgrade(&self.shared),
            generation,
        };
        let handle = self.driver().join(target, opts, sink);
        if self.shared.model.lock().unwrap().phase.active() {
            *self.shared.handle.lock().unwrap() = Some(Arc::from(handle));
        }
    }

    /// Runs a call window command.
    pub fn command(&self, c: CallCmd) {
        let now = Instant::now();
        let control = {
            let mut m = self.shared.model.lock().unwrap();
            match c {
                CallCmd::JoinNow => {
                    drop(m);
                    return self.join_now();
                }
                CallCmd::JoinChat(_) => None,
                CallCmd::Cancel => {
                    m.cancel();
                    None
                }
                CallCmd::PrejoinMic(on) => {
                    m.mic_on = on;
                    None
                }
                CallCmd::Mic(i) => m.pick_device(true, i),
                CallCmd::Speaker(i) => m.pick_device(false, i),
                CallCmd::ToggleMute => m.toggle_mute().map(Control::Mute),
                CallCmd::ToggleHand => m.toggle_hand().map(|up| {
                    if up {
                        Control::RaiseHand
                    } else {
                        Control::LowerHand
                    }
                }),
                CallCmd::ToggleHold => m.toggle_hold().map(Control::Hold),
                CallCmd::TransferOpen => m.transfer_opened().map(Control::Hold),
                CallCmd::Find(_) => None,
                CallCmd::Invite(mri) => m.invite(&mri),
                CallCmd::Transfer(mri) => m.transfer(&mri),
                CallCmd::Leave => m.leave(now).then_some(Control::Leave),
                CallCmd::Close => {
                    m.close();
                    None
                }
                CallCmd::Rejoin => {
                    drop(m);
                    let devices = self.driver().devices();
                    self.shared.model.lock().unwrap().rejoin(devices);
                    self.shared.push();
                    return;
                }
                CallCmd::DismissError => {
                    m.error.clear();
                    None
                }
                CallCmd::Captions(on) => {
                    let active = m.phase.active();
                    (active && m.set_captions(on)).then_some(Control::Captions(on))
                }
                CallCmd::Recording(start) => m.phase.active().then_some(Control::Recording(start)),
                CallCmd::Transcription(start) => {
                    m.phase.active().then_some(Control::Transcription(start))
                }
                CallCmd::DismissNotice => {
                    m.notice.clear();
                    None
                }
            }
        };
        if let Some(c) = control {
            let leave = c == Control::Leave;
            self.shared.control(c);
            if leave {
                self.shared.handle.lock().unwrap().take();
            }
        }
        self.shared.push();
    }

    /// A people search starts; pass its number to [`Self::found`].
    pub fn find_started(&self) -> u64 {
        self.shared.model.lock().unwrap().find_started()
    }

    /// The people search `seq` answered (dropped when a newer one started).
    pub fn found(&self, seq: u64, found: Vec<Found>) {
        if self.shared.model.lock().unwrap().found(seq, found) {
            self.shared.push();
        }
    }

    /// What the people search would have answered for the latest query (tests).
    #[cfg(test)]
    pub fn test_found(&self, found: Vec<Found>) {
        let seq = self.shared.model.lock().unwrap().find_seq;
        self.found(seq, found);
    }

    /// Demo mode: the fake driver with a timed script; `connected` also starts a connected
    /// meeting right away (`kelma --call-demo`).
    pub fn demo(&self, connected: bool) {
        let fake = FakeDriver::scripted(!connected);
        self.set_driver(Arc::new(fake));
        if connected {
            self.prejoin(
                "Weekly design review".into(),
                JoinTarget::Code {
                    id: "123456789".into(),
                    passcode: "demo".into(),
                },
                String::new(),
            );
            self.join_now();
        }
    }
}

/// The running manager, once kelma is signed in ([`CallManager::start`]); the 1:1 call flows
/// (`call_direct`) reach it here.
#[cfg(feature = "calls")]
static MANAGER: Mutex<Option<Arc<CallManager>>> = Mutex::new(None);

#[cfg(feature = "calls")]
pub fn manager() -> Option<Arc<CallManager>> {
    MANAGER.lock().unwrap().clone()
}

/// The entry points of the 1:1 call flows (`call_direct`).
#[cfg(feature = "calls")]
impl CallManager {
    /// Starts the window on a call that is already connected, run by `handle`. False while
    /// another call runs.
    fn adopt_handle(
        &self,
        kind: CallKind,
        start: impl FnOnce(EventSink) -> Box<dyn CallHandle>,
    ) -> bool {
        let devices = self.driver().devices();
        if !self
            .shared
            .model
            .lock()
            .unwrap()
            .adopted(kind, devices, Instant::now())
        {
            return false;
        }
        let generation = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let handle = start(EventSink {
            shared: Arc::downgrade(&self.shared),
            generation,
        });
        *self.shared.handle.lock().unwrap() = Some(Arc::from(handle));
        self.shared.publish_presence();
        self.shared.push();
        self.shared.tick();
        true
    }

    /// The entry point for a connected 1:1 call (answered or placed by the ringing flows): the
    /// manager runs it from here on (keepAlive, roster, controls, audio through cpal with echo
    /// cancellation, leave). Needs a tokio runtime. Errors while another call runs.
    pub fn adopt(
        &self,
        session: calling::signaling::CallSession,
        links: calling::Links,
        media: calling::media::MediaSession,
        events: calling::media::MediaEvents,
        kind: CallKind,
    ) -> anyhow::Result<()> {
        let (hub, shared) = (self.hub.clone(), self.shared.clone());
        let mut parts = Some((session, links, media, events));
        let adopted = self.adopt_handle(kind, |sink| {
            let (session, links, media, events) = parts.take().expect("taken once");
            let opts = shared.model.lock().unwrap().options();
            live::adopt(hub, session, links, media, events, opts, sink)
        });
        if !adopted {
            if let Some((_, _, media, _)) = parts {
                tokio::spawn(async move {
                    let _ = media.close().await;
                });
            }
            anyhow::bail!("{ALREADY_IN_CALL}");
        }
        Ok(())
    }

    /// A connected 1:1 call run by `fake` (UI tests).
    #[cfg(test)]
    pub fn adopt_fake(&self, fake: &FakeDriver, kind: CallKind) -> bool {
        let fake = fake.clone();
        self.adopt_handle(kind, move |sink| fake.adopt(sink))
    }

    /// Incoming call invitations, for the ringing flows.
    pub fn subscribe_invitations(
        &self,
    ) -> tokio::sync::mpsc::UnboundedReceiver<Box<calling::CallInvitation>> {
        self.hub.subscribe_invitations()
    }

    /// Call callbacks whose session is not the active call (a ringing call's cancel, an outgoing
    /// call's progress and acceptance) and other pushes.
    pub fn subscribe_unmatched(
        &self,
    ) -> tokio::sync::mpsc::UnboundedReceiver<calling::ngc_trouter::CallPush> {
        self.hub.subscribe_unmatched()
    }

    /// The calling trouter's surl (callback links are built on it), while connected.
    pub fn surl(&self) -> Option<String> {
        self.hub.surl()
    }

    /// The conversation service client (discovered at sign-in, else now).
    pub async fn ngc(&self) -> anyhow::Result<calling::signaling::NgcClient> {
        self.hub.ngc_client().await
    }
}

impl Engine {
    pub(super) async fn call_cmd(self: &Arc<Self>, c: CallCmd) -> anyhow::Result<()> {
        match c {
            CallCmd::JoinChat(thread) => {
                let title = {
                    let v = self.view.lock().unwrap();
                    if v.current.as_deref() == Some(thread.as_str()) {
                        v.current_title.clone()
                    } else {
                        String::new()
                    }
                };
                let title = if title.is_empty() {
                    "Meeting".to_owned()
                } else {
                    title
                };
                self.calls
                    .prejoin(title, JoinTarget::Chat(thread.clone()), thread);
            }
            CallCmd::Find(q) => {
                let seq = self.calls.find_started();
                let q = q.trim();
                if q.is_empty() {
                    self.calls.found(seq, Vec::new());
                    return Ok(());
                }
                // Tests answer with `CallManager::test_found`.
                if cfg!(test) {
                    return Ok(());
                }
                match self.client.search_people(&self.me, q).await {
                    Ok(people) => {
                        let found = people
                            .into_iter()
                            .filter(|p| p.mri != self.me)
                            .map(|p| Found {
                                subtitle: p.job_title.or(p.email).unwrap_or_default(),
                                mri: p.mri,
                                name: p.display_name,
                            })
                            .collect();
                        self.calls.found(seq, found);
                    }
                    // The box keeps what it showed.
                    Err(e) => tracing::warn!(%e, "people search for the call failed"),
                }
            }
            other => self.calls.command(other),
        }
        Ok(())
    }

    /// The calendar's Join for a meeting link: the pre-join screen. False when native calls are
    /// off, so the caller opens the link in the browser.
    pub(super) fn call_prejoin_link(&self, title: &str, url: &str) -> bool {
        if !self.calls.native() {
            return false;
        }
        let chat = ::calendar::meeting_chat_thread_id(url).unwrap_or_default();
        self.calls
            .prejoin(title.to_owned(), JoinTarget::Link(url.to_owned()), chat);
        true
    }

    /// "Join with an ID": the pre-join screen for that meeting ID and passcode.
    pub(super) fn call_prejoin_code(&self, id: &str, passcode: &str) -> bool {
        if !self.calls.native() {
            return false;
        }
        let id: String = id.chars().filter(|c| !c.is_whitespace()).collect();
        self.calls.prejoin(
            format!("Meeting {id}"),
            JoinTarget::Code {
                id,
                passcode: passcode.trim().to_owned(),
            },
            String::new(),
        );
        true
    }
}

/// A driver with no network or audio: it records what it was asked and reports what the test
/// (or the demo script) tells it to.
#[derive(Clone, Default)]
pub struct FakeDriver {
    /// Everything asked of it, as `join <target> mic=on|off`, `mute on`, `hand up`, `leave`, ...
    pub log: Arc<Mutex<Vec<String>>>,
    sink: Arc<Mutex<Option<EventSink>>>,
    /// Demo script: `Some(delayed)` plays connecting, lobby (when delayed) and connected.
    script: Option<bool>,
}

impl FakeDriver {
    /// Reports only what [`Self::emit`] sends.
    #[cfg(test)]
    pub fn manual() -> Self {
        Self::default()
    }

    /// Plays the demo meeting: a few seconds in the lobby when `delayed`, else straight in.
    pub fn scripted(delayed: bool) -> Self {
        FakeDriver {
            script: Some(delayed),
            ..Self::default()
        }
    }

    /// Sends an event as the running call would.
    #[cfg(test)]
    pub fn emit(&self, ev: Event) {
        let sink = self.sink.lock().unwrap().clone();
        if let Some(s) = sink {
            s.send(ev);
        }
    }

    /// Runs a connected 1:1 call as the live driver's `adopt` would (tests).
    #[cfg(test)]
    pub fn adopt(&self, sink: EventSink) -> Box<dyn CallHandle> {
        *self.sink.lock().unwrap() = Some(sink.clone());
        Box::new(FakeHandle {
            log: self.log.clone(),
            sink,
            demo: false,
        })
    }

    #[cfg(test)]
    pub fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn demo_people(name: &str) -> Vec<Person> {
        let p = |n: &str, muted, hand, lobby, role: &str| Person {
            name: n.into(),
            muted,
            hand,
            lobby,
            me: false,
            role: role.into(),
            ..Default::default()
        };
        vec![
            p("Amira Hassan", false, false, false, "organizer"),
            Person {
                name: name.into(),
                me: true,
                role: "attendee".into(),
                ..Default::default()
            },
            p("Daniel Park", true, true, false, "presenter"),
            p("Sara Ali", true, false, false, "attendee"),
            p("Omar Farouk", false, false, false, "attendee"),
            p("Guest (external)", false, false, true, ""),
        ]
    }
}

struct FakeHandle {
    log: Arc<Mutex<Vec<String>>>,
    sink: EventSink,
    /// The demo answers recording, transcription and captions as the service would.
    demo: bool,
}

fn start_stop(on: bool) -> &'static str {
    if on { "start" } else { "stop" }
}

impl CallHandle for FakeHandle {
    fn control(&self, c: Control) {
        let line = match &c {
            Control::Mute(m) => format!("mute {}", if *m { "on" } else { "off" }),
            Control::RaiseHand => "hand up".into(),
            Control::LowerHand => "hand down".into(),
            Control::Devices { mic, speaker } => format!(
                "devices {} {}",
                mic.as_deref().unwrap_or("default"),
                speaker.as_deref().unwrap_or("default")
            ),
            Control::Captions(on) => format!("captions {}", if *on { "on" } else { "off" }),
            Control::Recording(on) => format!("recording {}", start_stop(*on)),
            Control::Transcription(on) => format!("transcription {}", start_stop(*on)),
            Control::Hold(h) => format!("hold {}", if *h { "on" } else { "off" }),
            Control::AddPerson { mri, .. } => format!("add {mri}"),
            Control::Transfer { mri, .. } => format!("transfer {mri}"),
            Control::Leave => "leave".into(),
        };
        self.log.lock().unwrap().push(line);
        if self.demo {
            match c {
                Control::Recording(on) => self.sink.send(Event::Recording(on)),
                Control::Transcription(on) => self.sink.send(Event::Transcribing(on)),
                Control::Captions(true) => self.sink.send(Event::Caption(Caption {
                    id: "demo".into(),
                    speaker: "Amira Hassan".into(),
                    text: "Let's start with the design review.".into(),
                })),
                _ => {}
            }
        }
        if c == Control::Leave {
            self.sink.send(Event::Ended(YOU_LEFT_MEETING.into()));
        }
    }
}

impl CallDriver for FakeDriver {
    fn devices(&self) -> Devices {
        let d = |id: &str, name: &str| Device {
            id: Some(id.into()),
            name: name.into(),
        };
        Devices {
            mics: vec![
                d("mic-builtin", "Built-in Microphone"),
                d("mic-usb", "USB Headset Microphone"),
            ],
            speakers: vec![
                d("spk-builtin", "Built-in Speakers"),
                d("spk-usb", "USB Headset"),
            ],
            mic: 0,
            speaker: 0,
        }
    }

    fn join(&self, target: JoinTarget, opts: JoinOptions, sink: EventSink) -> Box<dyn CallHandle> {
        self.log.lock().unwrap().push(format!(
            "join {} mic={}",
            target.describe(),
            if opts.mic_on { "on" } else { "off" }
        ));
        *self.sink.lock().unwrap() = Some(sink.clone());
        sink.send(Event::Connecting);
        if let Some(delayed) = self.script {
            let (s, people) = (sink.clone(), Self::demo_people(&opts.name));
            let play = async move {
                if delayed {
                    tokio::time::sleep(Duration::from_millis(1200)).await;
                    s.send(Event::Lobby);
                    tokio::time::sleep(Duration::from_millis(2500)).await;
                }
                s.send(Event::Connected);
                s.send(Event::Roster(people));
                s.send(Event::Recording(true));
            };
            match tokio::runtime::Handle::try_current() {
                Ok(rt) if delayed => {
                    rt.spawn(play);
                }
                _ => {
                    sink.send(Event::Connected);
                    sink.send(Event::Roster(Self::demo_people(&opts.name)));
                    sink.send(Event::Recording(true));
                }
            }
        }
        Box::new(FakeHandle {
            log: self.log.clone(),
            sink,
            demo: self.script.is_some(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn devices() -> Devices {
        FakeDriver::manual().devices()
    }

    fn meeting() -> CallKind {
        CallKind::Meeting {
            title: "Standup".into(),
        }
    }

    fn ready() -> CallModel {
        let mut m = CallModel {
            my_name: "Me".into(),
            ..Default::default()
        };
        m.prejoin(
            meeting(),
            JoinTarget::Link("l".into()),
            "19:c".into(),
            devices(),
        )
        .unwrap();
        m
    }

    #[test]
    fn prejoin_then_join_then_lobby_then_connected() {
        let t0 = Instant::now();
        let mut m = ready();
        assert_eq!(m.phase, Phase::PreJoin);
        assert!(m.mic_on);
        m.mic_on = false;
        let (target, opts) = m.joining().unwrap();
        assert_eq!(target, JoinTarget::Link("l".into()));
        assert_eq!(opts.name, "Me");
        assert!(!opts.mic_on);
        assert_eq!(opts.mic.as_deref(), Some("mic-builtin"));
        assert_eq!(m.phase, Phase::Connecting);
        assert!(m.muted, "joining with the mic off joins muted");
        assert!(m.joining().is_none(), "join runs once");

        assert!(m.apply(Event::Lobby, t0));
        assert_eq!(m.phase, Phase::Lobby);
        // The admit re-offer: connecting again, then connected.
        assert!(m.apply(Event::Connecting, t0));
        assert!(m.apply(Event::Connected, t0));
        assert_eq!(m.phase, Phase::Connected);
        assert_eq!(m.data(t0 + Duration::from_secs(65)).timer, "01:05");
    }

    #[test]
    fn reconnecting_only_from_connected_and_the_timer_keeps_running() {
        let t0 = Instant::now();
        let mut m = ready();
        m.joining();
        assert!(!m.apply(Event::Reconnecting, t0), "not before connecting");
        m.apply(Event::Connected, t0);
        assert!(m.apply(Event::Reconnecting, t0 + Duration::from_secs(5)));
        assert_eq!(m.phase, Phase::Reconnecting);
        assert!(
            !m.apply(Event::Connecting, t0),
            "a reconnect is not a new join"
        );
        m.apply(Event::Connected, t0 + Duration::from_secs(9));
        assert_eq!(m.connected_at, Some(t0));
        assert_eq!(m.data(t0 + Duration::from_secs(3700)).timer, "1:01:40");
    }

    #[test]
    fn leave_ends_the_call_and_close_and_rejoin_follow() {
        let t0 = Instant::now();
        let mut m = ready();
        m.joining();
        m.apply(Event::Connected, t0);
        m.hand = true;
        assert!(m.leave(t0 + Duration::from_secs(30)));
        assert_eq!(m.phase, Phase::Ended);
        assert_eq!(m.ended, YOU_LEFT_MEETING);
        assert!(!m.hand);
        let d = m.data(t0 + Duration::from_secs(99));
        assert_eq!((d.phase, d.timer.as_str()), ("ended", "00:30"));
        assert!(
            !m.apply(Event::Connected, t0),
            "a stale event after the end"
        );
        assert!(!m.leave(t0));
        assert!(m.rejoin(devices()));
        assert_eq!(m.phase, Phase::PreJoin);
        assert!(m.cancel());
        assert_eq!(m.phase, Phase::Idle);
        assert!(!m.close());
    }

    #[test]
    fn the_service_ending_the_call_shows_why() {
        let mut m = ready();
        m.joining();
        assert!(m.apply(Event::Ended("The meeting ended.".into()), Instant::now()));
        assert_eq!(m.data(Instant::now()).ended, "The meeting ended.");
        assert_eq!(m.duration, None, "never connected");
        assert!(m.close());
        assert_eq!(m.data(Instant::now()).phase, "");
    }

    #[test]
    fn a_second_join_is_refused_while_a_call_runs() {
        let mut m = ready();
        m.joining();
        let r = m.prejoin(
            meeting(),
            JoinTarget::Chat("19:x".into()),
            String::new(),
            devices(),
        );
        assert_eq!(r, Err(ALREADY_IN_CALL));
        assert_eq!(m.phase, Phase::Connecting);
        assert_eq!(m.error, ALREADY_IN_CALL);
        assert!(!m.adopted(meeting(), devices(), Instant::now()));
    }

    #[test]
    fn controls_toggle_only_in_a_call() {
        let mut m = ready();
        assert_eq!(m.toggle_mute(), None);
        assert!(
            !m.mic_on,
            "on the pre-join screen mute flips the mic choice"
        );
        assert_eq!(m.toggle_hand(), None);
        m.mic_on = true;
        m.joining();
        assert_eq!(m.toggle_mute(), Some(true));
        assert_eq!(m.toggle_mute(), Some(false));
        assert_eq!(m.toggle_hand(), Some(true));
        assert_eq!(m.toggle_hand(), Some(false));
        assert_eq!(m.pick_device(true, 0), None, "already picked");
        assert_eq!(m.pick_device(true, 9), None, "no such device");
        assert_eq!(
            m.pick_device(false, 1),
            Some(Control::Devices {
                mic: Some("mic-builtin".into()),
                speaker: Some("spk-usb".into())
            })
        );
    }

    #[test]
    fn photos_are_looked_up_by_mri_for_the_people_who_have_one() {
        let p = |mri: &str| Person {
            mri: mri.into(),
            ..Default::default()
        };
        let px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(2, 2);
        let looked: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let found = photos(
            &[
                p("8:example-ada"),
                p(""),
                p("8:example-bo"),
                p("8:example-ada"),
            ],
            |mri| {
                looked.lock().unwrap().push(mri.to_owned());
                (mri == "8:example-ada").then(|| px.clone())
            },
        );
        assert_eq!(found.keys().collect::<Vec<_>>(), ["8:example-ada"]);
        assert_eq!(
            *looked.lock().unwrap(),
            ["8:example-ada", "8:example-bo"],
            "no MRI is not looked up, nobody twice"
        );
    }

    #[test]
    fn the_roster_shows_our_own_state_and_the_lobby_last() {
        let t0 = Instant::now();
        let mut m = ready();
        m.joining();
        m.apply(Event::Connected, t0);
        let p = |name: &str, lobby, me| Person {
            name: name.into(),
            lobby,
            me,
            ..Default::default()
        };
        m.apply(
            Event::Roster(vec![
                p("Guest", true, false),
                p("Me", false, true),
                p("Ada", false, false),
            ]),
            t0,
        );
        m.toggle_mute();
        m.toggle_hand();
        let d = m.data(t0);
        let names: Vec<&str> = d.people.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Me", "Ada", "Guest"]);
        assert!(d.people[0].muted && d.people[0].hand);
        assert_eq!((d.in_call, d.in_lobby), (2, 1));
        assert!(!m.apply(Event::Recording(false), t0));
        assert!(m.apply(Event::Recording(true), t0));
        assert!(m.data(t0).recording);
    }

    #[test]
    fn recording_shows_the_consent_notice_until_it_stops_or_is_dismissed() {
        let t0 = Instant::now();
        let mut m = ready();
        m.joining();
        m.apply(Event::Connected, t0);
        m.apply(Event::Recording(true), t0);
        assert_eq!(m.notice, RECORDING_NOTICE);
        assert!(!m.apply(Event::Recording(true), t0), "no change, no notice");
        m.apply(Event::Transcribing(true), t0);
        assert_eq!(m.notice, TRANSCRIPTION_NOTICE);
        m.apply(Event::Recording(false), t0);
        assert_eq!(m.notice, TRANSCRIPTION_NOTICE, "not the recording's");
        m.apply(Event::Transcribing(false), t0);
        assert_eq!(m.notice, "");
        m.apply(Event::Recording(true), t0);
        m.leave(t0);
        assert!(!m.recording && m.notice.is_empty());
    }

    #[test]
    fn captions_keep_the_latest_lines_and_only_while_on() {
        let t0 = Instant::now();
        let mut m = ready();
        m.joining();
        m.apply(Event::Connected, t0);
        let c = |id: &str, text: &str| {
            Event::Caption(Caption {
                id: id.into(),
                speaker: "Ada".into(),
                text: text.into(),
            })
        };
        assert!(!m.apply(c("u0", "ignored"), t0), "captions are off");
        m.set_captions(true);
        assert!(m.apply(c("u1", "a"), t0));
        assert!(m.apply(c("u1", "ab"), t0));
        assert!(!m.apply(c("u1", "ab"), t0));
        assert!(!m.apply(c("u9", "  "), t0), "empty");
        m.apply(c("u2", "b"), t0);
        m.apply(c("u3", "c"), t0);
        let texts: Vec<&str> = m.caption_lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["b", "c"]);
        assert!(m.apply(Event::Captions(false), t0));
        assert!(m.data(t0).caption_lines.is_empty());
    }

    #[test]
    fn the_more_commands_parse() {
        assert_eq!(
            CallCmd::parse("captions", "on"),
            Some(CallCmd::Captions(true))
        );
        assert_eq!(
            CallCmd::parse("captions", "off"),
            Some(CallCmd::Captions(false))
        );
        assert_eq!(
            CallCmd::parse("recording", "start"),
            Some(CallCmd::Recording(true))
        );
        assert_eq!(
            CallCmd::parse("transcription", "stop"),
            Some(CallCmd::Transcription(false))
        );
        assert_eq!(
            CallCmd::parse("dismiss-notice", ""),
            Some(CallCmd::DismissNotice)
        );
    }

    #[test]
    fn adopting_a_direct_call_starts_connected() {
        let t0 = Instant::now();
        let mut m = CallModel::default();
        assert!(m.adopted(
            CallKind::Direct {
                peer_name: "Ada".into(),
                chat_id: "19:a_b@unq.gbl.spaces".into(),
            },
            Devices::default(),
            t0
        ));
        let d = m.data(t0);
        assert_eq!(
            (d.phase, d.kind, d.title.as_str()),
            ("connected", "direct", "Ada")
        );
        assert_eq!(d.mics, ["System default"], "no device listed: the default");
        assert!(!d.can_rejoin);
        m.leave(t0);
        assert_eq!(m.ended, YOU_LEFT_CALL);
    }

    #[test]
    fn a_direct_call_holds_invites_and_transfers_people_from_the_search() {
        let t0 = Instant::now();
        let mut m = CallModel::default();
        let kind = CallKind::Direct {
            peer_name: "Ada".into(),
            chat_id: "19:a_b@unq.gbl.spaces".into(),
        };
        assert!(m.adopted(kind, Devices::default(), t0));
        assert_eq!(m.data(t0).chat_id, "19:a_b@unq.gbl.spaces");
        assert_eq!(m.toggle_hold(), Some(true));
        assert_eq!(m.toggle_hold(), Some(false));
        assert_eq!(m.transfer_opened(), Some(true), "Transfer holds first");
        assert_eq!(m.transfer_opened(), None, "already on hold");

        let bo = Found {
            mri: "8:orgid:b".into(),
            name: "Bo".into(),
            subtitle: String::new(),
        };
        let old = m.find_started();
        let seq = m.find_started();
        assert!(!m.found(old, vec![bo.clone()]), "a stale answer is dropped");
        assert!(m.found(seq, vec![bo.clone()]));
        assert_eq!(m.invite("8:orgid:nobody"), None);
        assert_eq!(
            m.invite("8:orgid:b"),
            Some(Control::AddPerson {
                mri: "8:orgid:b".into(),
                name: "Bo".into()
            })
        );
        assert_eq!(m.data(t0).invite_status, "Calling Bo…");
        assert!(m.found.is_empty());
        assert!(
            !m.found(seq, vec![bo.clone()]),
            "the pick ended that search"
        );
        let seq = m.find_started();
        m.found(seq, vec![bo]);
        assert_eq!(
            m.transfer("8:orgid:b"),
            Some(Control::Transfer {
                mri: "8:orgid:b".into(),
                name: "Bo".into()
            })
        );
        m.apply(Event::Held(false), t0);
        assert!(!m.data(t0).held);
        m.leave(t0);
        assert!(!m.held && m.invite_status.is_empty());
    }

    #[test]
    fn live_phase_follows_the_roster_and_the_media() {
        assert_eq!(live_phase(Link::Connected, true, false), Phase::Lobby);
        assert_eq!(
            live_phase(Link::Connecting, false, false),
            Phase::Connecting
        );
        assert_eq!(live_phase(Link::Connected, false, false), Phase::Connected);
        assert_eq!(
            live_phase(Link::Disconnected, false, true),
            Phase::Reconnecting
        );
        assert_eq!(live_phase(Link::New, false, true), Phase::Reconnecting);
    }

    #[test]
    fn recording_is_read_from_likely_shapes_only() {
        assert_eq!(
            recording_state(&json!({"a": {"isRecording": true}})),
            Some(true)
        );
        assert_eq!(
            recording_state(&json!({"meeting": [{"recordingStatus": "Started"}]})),
            Some(true)
        );
        assert_eq!(
            recording_state(&json!({"recordingState": "Stopped"})),
            Some(false)
        );
        assert_eq!(recording_state(&json!({"roster": {"x": 1}})), None);
    }

    #[test]
    fn commands_parse() {
        assert_eq!(
            CallCmd::parse("join-chat", "19:meeting_x@thread.v2"),
            Some(CallCmd::JoinChat("19:meeting_x@thread.v2".into()))
        );
        assert_eq!(CallCmd::parse("join-chat", "19:abc@thread.v2"), None);
        assert_eq!(
            CallCmd::parse("prejoin-mic", "off"),
            Some(CallCmd::PrejoinMic(false))
        );
        assert_eq!(CallCmd::parse("mic", "1"), Some(CallCmd::Mic(1)));
        assert_eq!(CallCmd::parse("mic", "x"), None);
        assert_eq!(CallCmd::parse("leave", ""), Some(CallCmd::Leave));
        assert_eq!(CallCmd::parse("hold", ""), Some(CallCmd::ToggleHold));
        assert_eq!(
            CallCmd::parse("find", "ada"),
            Some(CallCmd::Find("ada".into()))
        );
        assert_eq!(
            CallCmd::parse("invite", "8:orgid:a"),
            Some(CallCmd::Invite("8:orgid:a".into()))
        );
        assert_eq!(CallCmd::parse("invite", ""), None);
        assert_eq!(
            CallCmd::parse("transfer", "8:orgid:a"),
            Some(CallCmd::Transfer("8:orgid:a".into()))
        );
        assert_eq!(
            CallCmd::parse("react", ""),
            None,
            "no reaction API in the engine"
        );
    }

    #[test]
    fn without_the_engine_a_join_ends_at_once_and_joins_open_in_the_browser() {
        assert!(!Unavailable.native());
        let sink = EventSink::detached();
        Unavailable
            .join(JoinTarget::Link("l".into()), ready().options(), sink)
            .control(Control::Leave);
        assert_eq!(Unavailable.devices(), Devices::default());
    }

    #[test]
    fn clock_formats() {
        assert_eq!(clock(Duration::ZERO), "00:00");
        assert_eq!(clock(Duration::from_secs(599)), "09:59");
        assert_eq!(clock(Duration::from_secs(7322)), "2:02:02");
    }
}
