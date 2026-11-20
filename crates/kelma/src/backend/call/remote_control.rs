//! Remote control of a shared screen in a live call: "Give control" / "Request control" (see
//! `docs/protocol-calls.md`, Remote control). Only with `KELMA_REMOTE_CONTROL=1`.
//!
//! - We share: the share starts in a combined RemoteDesktop+ScreenCast portal session
//!   ([`super::share::Picked::injector`]), so one consent covers the screen and the input. A
//!   viewer's request shows "<name> wants control of your screen" (Allow / Deny); "Give control ›
//!   <person>" grants it directly. While controlled, their input is injected and any key pressed
//!   in our window (not one we just injected), or Escape twice, takes control back.
//! - Someone else shares: "Request control" on the stage; once granted, our pointer and keys
//!   over the stage go to them as normalized events.
//!
//! The state machine ([`Machine`]) is pure and tested. The wire ([`calling::remote_control`]) is
//! a GUESS: nothing of it has been seen from a Teams client.

use super::{ControlAction, ControlInput, Event};
use calling::remote_control::{ControlChannel, ControlMessage, InputEvent, MouseButton};
use calling::video::remote_input::InputInjector;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Whether calls offer remote control (`KELMA_REMOTE_CONTROL` set to anything but `0`).
pub fn wanted() -> bool {
    super::remote_control_wanted()
}

/// Two Escapes this close together take control back.
const DOUBLE_ESCAPE: Duration = Duration::from_millis(1000);
/// A key event this soon after we injected a key is taken for the injected one.
const ECHO: Duration = Duration::from_millis(300);

/// Where control stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    /// We asked the presenter for control.
    Requested { mri: String, name: String },
    /// A viewer asks us for control of our screen.
    Incoming { mri: String, name: String },
    /// We control the presenter's screen.
    Controlling { mri: String, name: String },
    /// Someone controls our screen.
    Controlled { mri: String, name: String },
}

impl Mode {
    /// What the call window shows: `CallStore.control-mode` and the other person's name.
    pub fn view(&self) -> (&'static str, String) {
        match self {
            Mode::Off => ("", String::new()),
            Mode::Requested { name, .. } => ("requested", name.clone()),
            Mode::Incoming { name, .. } => ("incoming", name.clone()),
            Mode::Controlling { name, .. } => ("controlling", name.clone()),
            Mode::Controlled { name, .. } => ("controlled", name.clone()),
        }
    }
}

/// What the call is doing, as the machine needs it.
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    /// We share a screen whose input we can drive.
    pub can_give: bool,
    /// Someone else presents: (MRI, name).
    pub presenter: Option<(String, String)>,
}

/// The outcome of one step: what to tell the other side, what to inject, what to show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Step {
    pub send: Option<ControlMessage>,
    pub inject: Option<InputEvent>,
    pub error: Option<String>,
}

impl Step {
    fn send(m: ControlMessage) -> Self {
        Step {
            send: Some(m),
            ..Default::default()
        }
    }
}

/// The control state machine. Pure: no I/O, so every transition is tested.
#[derive(Debug, Default)]
pub struct Machine {
    pub mode: Mode,
    last_escape: Option<Instant>,
    last_injected_key: Option<Instant>,
}

impl Machine {
    /// A local action. `me` is our MRI.
    pub fn local(&mut self, a: &ControlAction, me: &str, ctx: &Ctx, now: Instant) -> Step {
        let off = |m: &mut Mode, msg| {
            *m = Mode::Off;
            Step::send(msg)
        };
        // A key in our window while we are controlled: decided before the match borrows `mode`.
        let take_back_key = match (a, &self.mode) {
            (ControlAction::LocalKey(text), Mode::Controlled { .. }) => self.local_key(text, now),
            _ => false,
        };
        match (a, &self.mode) {
            (ControlAction::Request, Mode::Off) => match &ctx.presenter {
                Some((mri, name)) => {
                    self.mode = Mode::Requested {
                        mri: mri.clone(),
                        name: name.clone(),
                    };
                    Step::send(ControlMessage::Request { from: me.into() })
                }
                None => Step::default(),
            },
            (ControlAction::Release, Mode::Requested { .. } | Mode::Controlling { .. }) => {
                off(&mut self.mode, ControlMessage::Release)
            }
            (ControlAction::Give { mri, name }, Mode::Off | Mode::Incoming { .. })
                if ctx.can_give && mri != me =>
            {
                self.mode = Mode::Controlled {
                    mri: mri.clone(),
                    name: name.clone(),
                };
                Step::send(ControlMessage::Grant { to: mri.clone() })
            }
            (ControlAction::Give { .. }, _) if !ctx.can_give => Step {
                error: Some("Start sharing your screen to give control.".into()),
                ..Default::default()
            },
            (ControlAction::Answer(allow), Mode::Incoming { mri, name }) => {
                let (mri, name) = (mri.clone(), name.clone());
                if *allow && ctx.can_give {
                    self.mode = Mode::Controlled {
                        mri: mri.clone(),
                        name,
                    };
                    Step::send(ControlMessage::Grant { to: mri })
                } else {
                    off(&mut self.mode, ControlMessage::Deny { to: mri })
                }
            }
            (ControlAction::TakeBack, Mode::Controlled { .. }) => {
                off(&mut self.mode, ControlMessage::Revoke)
            }
            (ControlAction::LocalKey(_), Mode::Controlled { .. }) if take_back_key => {
                off(&mut self.mode, ControlMessage::Revoke)
            }
            (ControlAction::Input(i), Mode::Controlling { .. }) => match outgoing(i) {
                Some(ev) => Step::send(ControlMessage::Input(ev)),
                None => Step::default(),
            },
            _ => Step::default(),
        }
    }

    /// Whether a key pressed in our window takes control back: Escape twice, or any key that is
    /// not the echo of one we just injected.
    fn local_key(&mut self, text: &str, now: Instant) -> bool {
        if text == "\u{1b}" {
            let twice = self
                .last_escape
                .is_some_and(|t| now.saturating_duration_since(t) <= DOUBLE_ESCAPE);
            self.last_escape = Some(now);
            if twice {
                return true;
            }
        }
        !self
            .last_injected_key
            .is_some_and(|t| now.saturating_duration_since(t) <= ECHO)
    }

    /// A message from the other side. `me` is our MRI; `name_of` names an MRI from the roster.
    pub fn remote(
        &mut self,
        m: ControlMessage,
        me: &str,
        ctx: &Ctx,
        name_of: impl Fn(&str) -> String,
        now: Instant,
    ) -> Step {
        match (m, &self.mode) {
            (ControlMessage::Request { from }, Mode::Off) if ctx.can_give && from != me => {
                self.mode = Mode::Incoming {
                    name: name_of(&from),
                    mri: from,
                };
                Step::default()
            }
            // Granted what we asked for, or given control unasked by the presenter.
            (ControlMessage::Grant { to }, Mode::Requested { .. } | Mode::Off) if to == me => {
                match &ctx.presenter {
                    Some((mri, name)) => {
                        self.mode = Mode::Controlling {
                            mri: mri.clone(),
                            name: name.clone(),
                        };
                    }
                    None => self.mode = Mode::Off,
                }
                Step::default()
            }
            (ControlMessage::Deny { to }, Mode::Requested { name, .. }) if to == me => {
                let name = name.clone();
                self.mode = Mode::Off;
                Step {
                    error: Some(format!("{name} didn't give you control.")),
                    ..Default::default()
                }
            }
            (ControlMessage::Revoke, Mode::Controlling { .. } | Mode::Requested { .. }) => {
                self.mode = Mode::Off;
                Step::default()
            }
            (ControlMessage::Release, Mode::Controlled { .. } | Mode::Incoming { .. }) => {
                self.mode = Mode::Off;
                Step::default()
            }
            (ControlMessage::Input(ev), Mode::Controlled { .. }) => {
                if matches!(ev, InputEvent::Key { .. }) {
                    self.last_injected_key = Some(now);
                }
                Step {
                    inject: Some(ev),
                    ..Default::default()
                }
            }
            _ => Step::default(),
        }
    }

    /// The call changed: our share stopped, or the presenter we control (or asked) left. Control
    /// ends; the other side is told when it can still hear.
    pub fn context(&mut self, ctx: &Ctx) -> Step {
        let presenter = ctx.presenter.as_ref().map(|(m, _)| m.as_str());
        match &self.mode {
            Mode::Controlled { .. } | Mode::Incoming { .. } if !ctx.can_give => {
                self.mode = Mode::Off;
                Step::send(ControlMessage::Revoke)
            }
            Mode::Controlling { mri, .. } | Mode::Requested { mri, .. }
                if presenter != Some(mri.as_str()) =>
            {
                self.mode = Mode::Off;
                Step::default()
            }
            _ => Step::default(),
        }
    }
}

/// The controller's UI input as a wire event.
pub fn outgoing(i: &ControlInput) -> Option<InputEvent> {
    Some(match i {
        ControlInput::Move { x, y } => InputEvent::PointerMove {
            x: f64::from(*x.min(&10_000)) / 10_000.0,
            y: f64::from(*y.min(&10_000)) / 10_000.0,
        },
        ControlInput::Button { button, pressed } => InputEvent::PointerButton {
            button: MouseButton::from_index(*button)?,
            pressed: *pressed,
        },
        ControlInput::Scroll { dx, dy } => InputEvent::Axis {
            dx: f64::from(*dx),
            dy: f64::from(*dy),
        },
        ControlInput::Key { text, pressed } => InputEvent::Key {
            keysym: calling::remote_control::keysym_for_slint(text)?,
            pressed: *pressed,
        },
    })
}

/// One call's remote control: the machine, the injector task, and what the UI was told.
#[derive(Default)]
pub struct RemoteControl {
    pub machine: Machine,
    inject: Option<mpsc::UnboundedSender<InputEvent>>,
    injector_task: Option<JoinHandle<()>>,
    shown: Option<(&'static str, String)>,
    /// The `sequenceNumber` of the last message sent.
    sequence: u64,
}

impl RemoteControl {
    /// Our share's injector (`None` when the share stopped or has none). Injection runs on its
    /// own task so D-Bus round trips never hold the call loop.
    pub fn set_injector(&mut self, injector: Option<InputInjector>) {
        if let Some(t) = self.injector_task.take() {
            t.abort();
        }
        self.inject = None;
        if let Some(inj) = injector {
            let (tx, mut rx) = mpsc::unbounded_channel::<InputEvent>();
            self.inject = Some(tx);
            self.injector_task = Some(tokio::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    if let Err(e) = inj.inject(&ev).await {
                        tracing::warn!(%e, "remote control input not injected");
                    }
                }
            }));
        }
    }

    pub fn can_inject(&self) -> bool {
        self.inject.is_some()
    }

    pub fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// Carries a step out: injects, sends (through `channel`), reports an error.
    pub async fn apply(&mut self, step: Step, channel: &dyn ControlChannel) -> Vec<Event> {
        let mut out = Vec::new();
        if let (Some(ev), Some(tx)) = (step.inject, &self.inject) {
            let _ = tx.send(ev);
        }
        if let Some(m) = step.send {
            let quiet = matches!(m, ControlMessage::Input(_));
            if let Err(e) = channel.send(&m).await {
                tracing::warn!(%e, ?quiet, "remote control message not sent");
                if !quiet {
                    out.push(Event::Error(
                        "Couldn't reach the other side for control.".into(),
                    ));
                }
            }
        }
        if let Some(e) = step.error {
            out.push(Event::Error(e));
        }
        out
    }

    /// The control banner's state, when it changed since last shown.
    pub fn changed(&mut self) -> Option<Event> {
        let now = self.machine.mode.view();
        (self.shown.as_ref() != Some(&now)).then(|| {
            self.shown = Some(now.clone());
            Event::Control {
                mode: now.0,
                name: now.1,
            }
        })
    }

    pub fn stop(&mut self) {
        self.set_injector(None);
        self.machine.mode = Mode::Off;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "8:orgid:me";
    const ADA: &str = "8:orgid:ada";

    fn sharing() -> Ctx {
        Ctx {
            can_give: true,
            presenter: None,
        }
    }

    fn viewing() -> Ctx {
        Ctx {
            can_give: false,
            presenter: Some((ADA.into(), "Ada".into())),
        }
    }

    fn name(_: &str) -> String {
        "Ada".into()
    }

    #[test]
    fn a_viewer_requests_is_granted_controls_and_releases() {
        let (mut m, now) = (Machine::default(), Instant::now());
        let s = m.local(&ControlAction::Request, ME, &viewing(), now);
        assert_eq!(s.send, Some(ControlMessage::Request { from: ME.into() }));
        assert_eq!(m.mode.view(), ("requested", "Ada".into()));
        // A grant for someone else is not ours.
        m.remote(
            ControlMessage::Grant { to: "8:x".into() },
            ME,
            &viewing(),
            name,
            now,
        );
        assert_eq!(m.mode.view().0, "requested");
        m.remote(
            ControlMessage::Grant { to: ME.into() },
            ME,
            &viewing(),
            name,
            now,
        );
        assert_eq!(m.mode.view(), ("controlling", "Ada".into()));
        let mv = ControlAction::Input(ControlInput::Move { x: 5000, y: 2500 });
        assert_eq!(
            m.local(&mv, ME, &viewing(), now).send,
            Some(ControlMessage::Input(InputEvent::PointerMove {
                x: 0.5,
                y: 0.25
            }))
        );
        let s = m.local(&ControlAction::Release, ME, &viewing(), now);
        assert_eq!(s.send, Some(ControlMessage::Release));
        assert_eq!(m.mode, Mode::Off);
        // Input with no control goes nowhere.
        assert_eq!(m.local(&mv, ME, &viewing(), now), Step::default());
    }

    #[test]
    fn a_denied_request_says_so_and_a_revoke_ends_control() {
        let (mut m, now) = (Machine::default(), Instant::now());
        m.local(&ControlAction::Request, ME, &viewing(), now);
        let s = m.remote(
            ControlMessage::Deny { to: ME.into() },
            ME,
            &viewing(),
            name,
            now,
        );
        assert_eq!(s.error.as_deref(), Some("Ada didn't give you control."));
        assert_eq!(m.mode, Mode::Off);
        m.remote(
            ControlMessage::Grant { to: ME.into() },
            ME,
            &viewing(),
            name,
            now,
        );
        assert_eq!(m.mode.view().0, "controlling", "given unasked");
        m.remote(ControlMessage::Revoke, ME, &viewing(), name, now);
        assert_eq!(m.mode, Mode::Off);
        // Nobody presents: nothing to ask.
        let s = m.local(&ControlAction::Request, ME, &Ctx::default(), now);
        assert_eq!((s.send, &m.mode), (None, &Mode::Off));
    }

    #[test]
    fn the_presenter_allows_or_denies_a_request() {
        let (mut m, now) = (Machine::default(), Instant::now());
        m.remote(
            ControlMessage::Request { from: ADA.into() },
            ME,
            &sharing(),
            name,
            now,
        );
        assert_eq!(m.mode.view(), ("incoming", "Ada".into()));
        let s = m.local(&ControlAction::Answer(false), ME, &sharing(), now);
        assert_eq!(s.send, Some(ControlMessage::Deny { to: ADA.into() }));
        assert_eq!(m.mode, Mode::Off);
        m.remote(
            ControlMessage::Request { from: ADA.into() },
            ME,
            &sharing(),
            name,
            now,
        );
        let s = m.local(&ControlAction::Answer(true), ME, &sharing(), now);
        assert_eq!(s.send, Some(ControlMessage::Grant { to: ADA.into() }));
        assert_eq!(m.mode.view(), ("controlled", "Ada".into()));
        // Their input is injected, only while they control.
        let ev = InputEvent::PointerButton {
            button: MouseButton::Left,
            pressed: true,
        };
        let s = m.remote(ControlMessage::Input(ev), ME, &sharing(), name, now);
        assert_eq!(s.inject, Some(ev));
        let s = m.local(&ControlAction::TakeBack, ME, &sharing(), now);
        assert_eq!(s.send, Some(ControlMessage::Revoke));
        let s = m.remote(ControlMessage::Input(ev), ME, &sharing(), name, now);
        assert_eq!(s.inject, None);
        // A request while we do not share is not shown.
        m.remote(
            ControlMessage::Request { from: ADA.into() },
            ME,
            &viewing(),
            name,
            now,
        );
        assert_eq!(m.mode, Mode::Off);
    }

    #[test]
    fn give_control_needs_a_controllable_share() {
        let (mut m, now) = (Machine::default(), Instant::now());
        let give = ControlAction::Give {
            mri: ADA.into(),
            name: "Ada".into(),
        };
        let s = m.local(&give, ME, &viewing(), now);
        assert!(s.error.is_some() && s.send.is_none());
        let s = m.local(&give, ME, &sharing(), now);
        assert_eq!(s.send, Some(ControlMessage::Grant { to: ADA.into() }));
        assert_eq!(m.mode.view(), ("controlled", "Ada".into()));
        // Our share stopped: control ends and they are told.
        let s = m.context(&Ctx::default());
        assert_eq!(s.send, Some(ControlMessage::Revoke));
        assert_eq!(m.mode, Mode::Off);
    }

    #[test]
    fn a_local_key_takes_control_back_but_not_the_echo_of_an_injected_one() {
        let (mut m, t0) = (Machine::default(), Instant::now());
        m.mode = Mode::Controlled {
            mri: ADA.into(),
            name: "Ada".into(),
        };
        let key = InputEvent::Key {
            keysym: 0x61,
            pressed: true,
        };
        m.remote(ControlMessage::Input(key), ME, &sharing(), name, t0);
        let echo = ControlAction::LocalKey("a".into());
        assert_eq!(m.local(&echo, ME, &sharing(), t0 + ms(50)).send, None);
        // Escape twice works even right after an injected key.
        let esc = ControlAction::LocalKey("\u{1b}".into());
        assert_eq!(m.local(&esc, ME, &sharing(), t0 + ms(100)).send, None);
        let s = m.local(&esc, ME, &sharing(), t0 + ms(200));
        assert_eq!(s.send, Some(ControlMessage::Revoke));
        // Any key of our own (long after the last injected one) takes back too.
        m.mode = Mode::Controlled {
            mri: ADA.into(),
            name: "Ada".into(),
        };
        let s = m.local(&echo, ME, &sharing(), t0 + ms(2000));
        assert_eq!(s.send, Some(ControlMessage::Revoke));
    }

    #[test]
    fn control_of_a_presenter_who_stopped_ends_quietly() {
        let (mut m, now) = (Machine::default(), Instant::now());
        m.remote(
            ControlMessage::Grant { to: ME.into() },
            ME,
            &viewing(),
            name,
            now,
        );
        assert_eq!(m.context(&viewing()), Step::default());
        assert_eq!(m.mode.view().0, "controlling");
        assert_eq!(m.context(&Ctx::default()), Step::default());
        assert_eq!(m.mode, Mode::Off);
    }

    #[test]
    fn ui_input_becomes_wire_events() {
        assert_eq!(
            outgoing(&ControlInput::Move { x: 20000, y: 0 }),
            Some(InputEvent::PointerMove { x: 1.0, y: 0.0 })
        );
        assert_eq!(
            outgoing(&ControlInput::Key {
                text: "\n".into(),
                pressed: true
            }),
            Some(InputEvent::Key {
                keysym: 0xff0d,
                pressed: true
            })
        );
        assert_eq!(
            outgoing(&ControlInput::Button {
                button: 9,
                pressed: true
            }),
            None
        );
        assert_eq!(
            outgoing(&ControlInput::Scroll { dx: 0, dy: -40 }),
            Some(InputEvent::Axis { dx: 0.0, dy: -40.0 })
        );
    }

    #[test]
    fn the_banner_hears_only_changes() {
        let mut rc = RemoteControl::default();
        assert_eq!(
            rc.changed(),
            Some(Event::Control {
                mode: "",
                name: String::new()
            })
        );
        assert_eq!(rc.changed(), None);
        rc.machine.mode = Mode::Incoming {
            mri: ADA.into(),
            name: "Ada".into(),
        };
        assert_eq!(
            rc.changed(),
            Some(Event::Control {
                mode: "incoming",
                name: "Ada".into()
            })
        );
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }
}
