//! Remote control of a shared screen: Teams' "Give control" / "Request control".
//!
//! **THE WIRE FORMAT IN THIS MODULE IS GUESSED. NOTHING HERE HAS BEEN SEEN ON THE WIRE.** See
//! `docs/protocol-calls.md` (Remote control) for what is known and what needs a capture. In
//! short:
//! - known: a presenter's content-sharing session answer names a `takeControl` and an
//!   `updateSessionState` link (teams-lite notes, facts only), and rusty-teams' roadmap (MIT)
//!   says control rides the meeting's `main-channel` data channel with "action codes 0-11";
//! - guessed: every body below, the data id, the action numbering, the JSON payload, and which
//!   message goes over HTTP and which over the data channel.
//!
//! Everything is behind [`ControlChannel`], so the guesses can be replaced by the captured
//! format in one place. kelma turns the feature on only with `KELMA_REMOTE_CONTROL=1`.

use crate::Result;
use crate::error::Error;
use crate::media::DataSender;
use crate::signaling::content_sharing::SharingSession;
use crate::signaling::{CallSession, NgcClient};
use crate::types::Links;
use serde_json::{Value, json};

/// The data id control messages use on the data channel. UNVERIFIED: a guess (captions are 3).
pub const CONTROL_DATA_ID: u8 = 9;
const FIXED_HEADER: usize = 11;
const START_FLAG: u8 = 0x80;
/// The first header byte of a frame (the high nibble as rusty-teams' test frames write it).
const HEADER_FLAGS: u8 = 0x10;

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    /// The button as Slint numbers it (`PointerEventButton`: 1 left, 2 right, 3 middle).
    pub fn from_index(i: u8) -> Option<MouseButton> {
        match i {
            1 => Some(MouseButton::Left),
            2 => Some(MouseButton::Right),
            3 => Some(MouseButton::Middle),
            _ => None,
        }
    }

    fn index(self) -> u8 {
        match self {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 3,
        }
    }

    /// The Linux evdev code (`BTN_LEFT` 0x110, `BTN_RIGHT` 0x111, `BTN_MIDDLE` 0x112), as the
    /// RemoteDesktop portal's `NotifyPointerButton` takes it.
    pub fn evdev(self) -> i32 {
        match self {
            MouseButton::Left => 0x110,
            MouseButton::Right => 0x111,
            MouseButton::Middle => 0x112,
        }
    }
}

/// One input event of the controller, on the presenter's shared picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// The pointer at a point of the picture, each coordinate 0..=1 from the top left.
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerButton {
        button: MouseButton,
        pressed: bool,
    },
    /// Scrolling, in pixels (positive: down, right).
    Axis {
        dx: f64,
        dy: f64,
    },
    /// A key, as an X keysym (what the RemoteDesktop portal's `NotifyKeyboardKeysym` takes).
    Key {
        keysym: u32,
        pressed: bool,
    },
}

/// What the presenter and a viewer tell each other.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
    /// A viewer (`from`, an MRI) asks the presenter for control.
    Request {
        from: String,
    },
    /// The presenter gives control to `to`.
    Grant {
        to: String,
    },
    /// The presenter turns `to`'s request down.
    Deny {
        to: String,
    },
    /// The presenter takes control back.
    Revoke,
    /// The controller gives control back (or a viewer withdraws its request).
    Release,
    Input(InputEvent),
}

/// The action codes of the data-channel payload. UNVERIFIED: rusty-teams says there are twelve
/// (0-11) and names none of them; this numbering is INVENTED to fill that range.
mod action {
    pub const REQUEST: u64 = 0;
    pub const GRANT: u64 = 1;
    pub const DENY: u64 = 2;
    pub const REVOKE: u64 = 3;
    pub const RELEASE: u64 = 4;
    pub const POINTER_MOVE: u64 = 5;
    pub const BUTTON_DOWN: u64 = 6;
    pub const BUTTON_UP: u64 = 7;
    pub const AXIS: u64 = 8;
    pub const KEY_DOWN: u64 = 9;
    pub const KEY_UP: u64 = 10;
}

/// The message as the data channel's JSON payload (UNVERIFIED shape).
pub fn encode(msg: &ControlMessage) -> Value {
    use action::*;
    let body = match msg {
        ControlMessage::Request { from } => json!({"action": REQUEST, "participant": from}),
        ControlMessage::Grant { to } => json!({"action": GRANT, "participant": to}),
        ControlMessage::Deny { to } => json!({"action": DENY, "participant": to}),
        ControlMessage::Revoke => json!({"action": REVOKE}),
        ControlMessage::Release => json!({"action": RELEASE}),
        ControlMessage::Input(InputEvent::PointerMove { x, y }) => {
            json!({"action": POINTER_MOVE, "x": x, "y": y})
        }
        ControlMessage::Input(InputEvent::PointerButton { button, pressed }) => json!({
            "action": if *pressed { BUTTON_DOWN } else { BUTTON_UP },
            "button": button.index(),
        }),
        ControlMessage::Input(InputEvent::Axis { dx, dy }) => {
            json!({"action": AXIS, "dx": dx, "dy": dy})
        }
        ControlMessage::Input(InputEvent::Key { keysym, pressed }) => json!({
            "action": if *pressed { KEY_DOWN } else { KEY_UP },
            "keysym": keysym,
        }),
    };
    json!({ "remoteControl": body })
}

/// Reads a message back from [`encode`]'s shape (from the data channel, or anywhere in a session
/// callback's body). `None` for anything else.
pub fn decode(v: &Value) -> Option<ControlMessage> {
    use action::*;
    let b = find_key(v, "remoteControl")?;
    let who = || b["participant"].as_str().map(str::to_owned);
    let f = |k: &str| b[k].as_f64();
    let unit = |k: &str| f(k).filter(|n| n.is_finite()).map(|n| n.clamp(0.0, 1.0));
    Some(match b["action"].as_u64()? {
        REQUEST => ControlMessage::Request { from: who()? },
        GRANT => ControlMessage::Grant { to: who()? },
        DENY => ControlMessage::Deny { to: who()? },
        REVOKE => ControlMessage::Revoke,
        RELEASE => ControlMessage::Release,
        POINTER_MOVE => ControlMessage::Input(InputEvent::PointerMove {
            x: unit("x")?,
            y: unit("y")?,
        }),
        a @ (BUTTON_DOWN | BUTTON_UP) => ControlMessage::Input(InputEvent::PointerButton {
            button: MouseButton::from_index(u8::try_from(b["button"].as_u64()?).ok()?)?,
            pressed: a == BUTTON_DOWN,
        }),
        AXIS => ControlMessage::Input(InputEvent::Axis {
            dx: f("dx").filter(|n| n.is_finite())?,
            dy: f("dy").filter(|n| n.is_finite())?,
        }),
        a @ (KEY_DOWN | KEY_UP) => ControlMessage::Input(InputEvent::Key {
            keysym: u32::try_from(b["keysym"].as_u64()?).ok()?,
            pressed: a == KEY_DOWN,
        }),
        _ => return None,
    })
}

fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m
            .get(key)
            .filter(|x| x.is_object())
            .or_else(|| m.values().find_map(|c| find_key(c, key))),
        Value::Array(a) => a.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

/// One whole message as a single data-channel frame (the framing captions arrive in:
/// [`crate::captions::Depacketizer`]): an 11-byte header, then the payload.
pub fn packetize(data_id: u8, sequence: u16, source_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(FIXED_HEADER + payload.len());
    f.push(HEADER_FLAGS);
    f.push(FIXED_HEADER as u8);
    f.push((data_id & 0x3f) | START_FLAG);
    f.extend_from_slice(&sequence.to_be_bytes());
    f.push(0); // no more fragments
    f.extend_from_slice(&source_id.to_be_bytes());
    f.push(0);
    f.extend_from_slice(payload);
    f
}

/// What the session callbacks say about control, in the shape [`session_body`] posts
/// (UNVERIFIED): `contentSharing.sessionState.controlState` with the `controller`, or a
/// `controlAction: "request"` from `participants.from`. Also reads [`encode`]'s shape.
pub fn parse_session_message(body: &Value) -> Option<ControlMessage> {
    if let Some(m) = decode(body) {
        return Some(m);
    }
    if let Some(state) = find_key(body, "sessionState") {
        let who = state["controller"].as_str().map(str::to_owned);
        return match state["controlState"].as_str()? {
            "granted" => Some(ControlMessage::Grant { to: who? }),
            "denied" => Some(ControlMessage::Deny { to: who? }),
            "revoked" => Some(ControlMessage::Revoke),
            "released" => Some(ControlMessage::Release),
            _ => None,
        };
    }
    let sharing = find_key(body, "contentSharing")?;
    match sharing["controlAction"].as_str()? {
        "request" => Some(ControlMessage::Request {
            from: body.pointer("/participants/from/id")?.as_str()?.to_owned(),
        }),
        "release" => Some(ControlMessage::Release),
        _ => None,
    }
}

/// The HTTP body of a session message (UNVERIFIED shape), or `None` for input, which only the
/// data channel carries.
pub fn session_body(s: &CallSession, msg: &ControlMessage, sequence: u64) -> Option<Value> {
    let state =
        |control: &str, who: Option<&str>| json!({"controlState": control, "controller": who});
    let sharing = match msg {
        ControlMessage::Request { .. } => {
            json!({"sequenceNumber": sequence, "controlAction": "request"})
        }
        ControlMessage::Release => json!({"sequenceNumber": sequence, "controlAction": "release"}),
        ControlMessage::Grant { to } => {
            json!({"sequenceNumber": sequence, "sessionState": state("granted", Some(to))})
        }
        ControlMessage::Deny { to } => {
            json!({"sequenceNumber": sequence, "sessionState": state("denied", Some(to))})
        }
        ControlMessage::Revoke => {
            json!({"sequenceNumber": sequence, "sessionState": state("revoked", None)})
        }
        ControlMessage::Input(_) => return None,
    };
    Some(json!({"participants": {"from": s.me.wire()}, "contentSharing": sharing}))
}

/// X keysyms of the keys Slint names with a private character (`slint::platform::Key`), and of
/// plain text: Latin-1 is its own keysym, the rest of Unicode is `0x0100_0000 + code point`.
pub fn keysym_for_slint(text: &str) -> Option<u32> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let special = match c {
        '\u{0008}' => 0xff08,                                            // BackSpace
        '\u{0009}' => 0xff09,                                            // Tab
        '\u{000a}' => 0xff0d,                                            // Return
        '\u{001b}' => 0xff1b,                                            // Escape
        '\u{0019}' => 0xfe20,                                            // ISO_Left_Tab (Backtab)
        '\u{007f}' => 0xffff,                                            // Delete
        '\u{0010}' => 0xffe1,                                            // Shift_L
        '\u{0011}' => 0xffe3,                                            // Control_L
        '\u{0012}' => 0xffe9,                                            // Alt_L
        '\u{0013}' => 0xff7e,                                            // Mode_switch (AltGr)
        '\u{0014}' => 0xffe5,                                            // Caps_Lock
        '\u{0015}' => 0xffe2,                                            // Shift_R
        '\u{0016}' => 0xffe4,                                            // Control_R
        '\u{0017}' => 0xffeb,                                            // Super_L (Meta)
        '\u{0018}' => 0xffec,                                            // Super_R
        '\u{f700}' => 0xff52,                                            // Up
        '\u{f701}' => 0xff54,                                            // Down
        '\u{f702}' => 0xff51,                                            // Left
        '\u{f703}' => 0xff53,                                            // Right
        '\u{f727}' => 0xff63,                                            // Insert
        '\u{f729}' => 0xff50,                                            // Home
        '\u{f72b}' => 0xff57,                                            // End
        '\u{f72c}' => 0xff55,                                            // Page_Up
        '\u{f72d}' => 0xff56,                                            // Page_Down
        '\u{f735}' => 0xff67,                                            // Menu
        f @ '\u{f704}'..='\u{f71b}' => 0xffbe + (u32::from(f) - 0xf704), // F1..F24
        _ => 0,
    };
    if special != 0 {
        return Some(special);
    }
    match u32::from(c) {
        0x20..=0x7e | 0xa0..=0xff => Some(u32::from(c)),
        n if n > 0xff && !(0xf700..=0xf8ff).contains(&n) => Some(0x0100_0000 + n),
        _ => None,
    }
}

/// A normalized point of the shared picture in the stream's own coordinates (what
/// `NotifyPointerMotionAbsolute` takes), clamped inside it.
pub fn stream_point(x: f64, y: f64, size: (u32, u32)) -> (f64, f64) {
    let clamp = |v: f64, max: u32| {
        let max = f64::from(max.max(1)) - 1.0;
        if v.is_finite() {
            (v.clamp(0.0, 1.0) * (max + 1.0)).min(max)
        } else {
            0.0
        }
    };
    (clamp(x, size.0), clamp(y, size.1))
}

/// Sends control messages to the other side. One trait, so the guessed transport can be swapped
/// for the captured one (or a fake in tests).
#[async_trait::async_trait]
pub trait ControlChannel: Send + Sync {
    async fn send(&self, msg: &ControlMessage) -> Result<()>;
}

/// The best guess at Teams' transport (UNVERIFIED):
/// - the presenter's grant / deny / revoke: `POST <updateSessionState>` of its own session;
/// - a viewer's request / release: `POST <takeControl>` from the presenter's roster entry, when
///   the roster names one;
/// - input, and anything without a link: the data channel, data id [`CONTROL_DATA_ID`].
pub struct NgcControlChannel {
    pub ngc: NgcClient,
    pub s: CallSession,
    /// Our own content-sharing session, when we present.
    pub own: Option<SharingSession>,
    /// The presenter's `contentSharing.links` from the roster, when we view.
    pub presenter_links: Links,
    pub data: Option<DataSender>,
    /// The `sequenceNumber` of this message.
    pub sequence: u64,
}

impl NgcControlChannel {
    async fn send_data(&self, msg: &ControlMessage) -> Result<()> {
        let data = self
            .data
            .as_ref()
            .ok_or_else(|| Error::Protocol("this call has no data channel".into()))?;
        let payload = serde_json::to_vec(&encode(msg))?;
        let frame = packetize(CONTROL_DATA_ID, self.sequence as u16, 0, &payload);
        data.send(&frame).await
    }
}

#[async_trait::async_trait]
impl ControlChannel for NgcControlChannel {
    async fn send(&self, msg: &ControlMessage) -> Result<()> {
        let Some(body) = session_body(&self.s, msg, self.sequence) else {
            return self.send_data(msg).await;
        };
        let presenter_side = matches!(
            msg,
            ControlMessage::Grant { .. } | ControlMessage::Deny { .. } | ControlMessage::Revoke
        );
        if presenter_side
            && let Some(own) = &self.own
            && let Some(url) = own.update_session_state()
        {
            self.ngc
                .post_content_sharing("contentSharingControl", &self.s, url, own, &body)
                .await?;
            return Ok(());
        }
        if !presenter_side && let Some(url) = self.presenter_links.get("takeControl") {
            self.ngc
                .send(
                    "contentSharingTakeControl",
                    &self.s,
                    reqwest::Method::POST,
                    url,
                    Some(&body),
                )
                .await?;
            return Ok(());
        }
        self.send_data(msg).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::captions::Depacketizer;

    fn all() -> Vec<ControlMessage> {
        vec![
            ControlMessage::Request {
                from: "8:orgid:a".into(),
            },
            ControlMessage::Grant {
                to: "8:orgid:a".into(),
            },
            ControlMessage::Deny {
                to: "8:orgid:a".into(),
            },
            ControlMessage::Revoke,
            ControlMessage::Release,
            ControlMessage::Input(InputEvent::PointerMove { x: 0.25, y: 0.75 }),
            ControlMessage::Input(InputEvent::PointerButton {
                button: MouseButton::Right,
                pressed: true,
            }),
            ControlMessage::Input(InputEvent::PointerButton {
                button: MouseButton::Left,
                pressed: false,
            }),
            ControlMessage::Input(InputEvent::Axis { dx: 0.0, dy: -30.0 }),
            ControlMessage::Input(InputEvent::Key {
                keysym: 0xff0d,
                pressed: true,
            }),
            ControlMessage::Input(InputEvent::Key {
                keysym: 0x61,
                pressed: false,
            }),
        ]
    }

    #[test]
    fn every_message_survives_the_payload_and_the_framing() {
        for (i, m) in all().into_iter().enumerate() {
            assert_eq!(decode(&encode(&m)).as_ref(), Some(&m), "{m:?}");
            let payload = serde_json::to_vec(&encode(&m)).unwrap();
            let frame = packetize(CONTROL_DATA_ID, i as u16, 7, &payload);
            let msg = Depacketizer::default()
                .push(&frame)
                .expect("one whole frame");
            assert_eq!(msg.data_id, CONTROL_DATA_ID);
            assert_eq!(msg.source_id, 7);
            let back: Value = serde_json::from_slice(&msg.payload).unwrap();
            assert_eq!(decode(&back), Some(m));
        }
    }

    #[test]
    fn a_pointer_outside_the_picture_is_clamped_and_junk_is_refused() {
        let far = json!({"remoteControl": {"action": 5, "x": 3.0, "y": -1.0}});
        assert_eq!(
            decode(&far),
            Some(ControlMessage::Input(InputEvent::PointerMove {
                x: 1.0,
                y: 0.0
            }))
        );
        assert_eq!(decode(&json!({"remoteControl": {"action": 99}})), None);
        assert_eq!(
            decode(&json!({"remoteControl": {"action": 6, "button": 9}})),
            None
        );
        assert_eq!(decode(&json!({"other": 1})), None);
    }

    #[test]
    fn session_messages_round_trip_through_their_http_bodies() {
        let s = CallSession::new("https://t.example.invalid/", "8:orgid:me", "Me");
        for m in all()
            .into_iter()
            .filter(|m| !matches!(m, ControlMessage::Input(_)))
        {
            let body = session_body(&s, &m, 3).unwrap();
            assert_eq!(body["contentSharing"]["sequenceNumber"], 3);
            let back = parse_session_message(&body).unwrap();
            match m {
                // The request names who sent it: the body's `participants.from`.
                ControlMessage::Request { .. } => assert_eq!(
                    back,
                    ControlMessage::Request {
                        from: "8:orgid:me".into()
                    }
                ),
                m => assert_eq!(back, m),
            }
        }
        let input = ControlMessage::Input(InputEvent::Axis { dx: 1.0, dy: 0.0 });
        assert!(
            session_body(&s, &input, 1).is_none(),
            "input rides the data channel"
        );
    }

    #[test]
    fn slint_keys_become_x_keysyms() {
        assert_eq!(keysym_for_slint("a"), Some(0x61));
        assert_eq!(keysym_for_slint("A"), Some(0x41));
        assert_eq!(keysym_for_slint(" "), Some(0x20));
        assert_eq!(keysym_for_slint("é"), Some(0xe9));
        assert_eq!(keysym_for_slint("ع"), Some(0x0100_0639));
        assert_eq!(keysym_for_slint("\n"), Some(0xff0d));
        assert_eq!(keysym_for_slint("\u{1b}"), Some(0xff1b));
        assert_eq!(keysym_for_slint("\u{f700}"), Some(0xff52));
        assert_eq!(keysym_for_slint("\u{f704}"), Some(0xffbe), "F1");
        assert_eq!(keysym_for_slint("\u{f70f}"), Some(0xffc9), "F12");
        assert_eq!(keysym_for_slint(""), None);
        assert_eq!(keysym_for_slint("ab"), None);
        assert_eq!(keysym_for_slint("\u{f72e}"), None, "unnamed private keys");
    }

    #[test]
    fn a_normalized_point_lands_inside_the_stream() {
        assert_eq!(stream_point(0.0, 0.0, (1920, 1080)), (0.0, 0.0));
        assert_eq!(stream_point(0.5, 0.5, (1920, 1080)), (960.0, 540.0));
        assert_eq!(stream_point(1.0, 1.0, (1920, 1080)), (1919.0, 1079.0));
        assert_eq!(stream_point(-2.0, 9.0, (100, 50)), (0.0, 49.0));
        assert_eq!(stream_point(f64::NAN, 0.5, (100, 50)), (0.0, 25.0));
    }

    #[test]
    fn buttons_map_to_evdev_and_back_from_slint() {
        assert_eq!(
            MouseButton::from_index(1).map(MouseButton::evdev),
            Some(0x110)
        );
        assert_eq!(
            MouseButton::from_index(2).map(MouseButton::evdev),
            Some(0x111)
        );
        assert_eq!(
            MouseButton::from_index(3).map(MouseButton::evdev),
            Some(0x112)
        );
        assert_eq!(MouseButton::from_index(0), None);
    }
}
