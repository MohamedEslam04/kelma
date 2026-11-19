//! Native Teams calling for kelma (plans/meetings.md M5, plans/calls.md K3.2).
//!
//! Layers, bottom up:
//! - [`ngc_trouter`]: a trouter connection of our own registered as `NextGenCalling`, which is
//!   where call invitations and call/conversation callbacks are pushed (the Teams web client also
//!   runs a separate trouter for its calling stack).
//! - [`signaling`]: the NGC conversation service (`epconv`): place, join, and every later step
//!   POSTed to a `link` the service handed back.
//! - [`invitation`]: parsing an incoming `callInvitation` push.
//! - [`incoming`]: answering one, with or without the caller's offer in the ring.
//! - [`relay`]: TURN credentials from `trap/tokens`.
//! - [`media`]: one WebRTC peer (webrtc-rs) speaking Opus over ICE/TURN + SRTP.
//! - [`audio`]: microphone/speaker I/O and the Opus codec.
//! - [`history`]: call history from the `48:calllogs` system thread.
//! - [`emergency`]: emergency short codes kelma refuses to dial.
//! - [`captions`]: live caption text from the media data channel.
//!
//! Protocol sources: rusty-teams `crates/calling` and eisbaw/ost `src/calling` (MIT, code may be
//! reused with attribution); mautrix-teams `pkg/msteams/calls.go` (AGPL) and teams-lite
//! `NATIVE-CALLING.md` (no licence) are read for protocol facts only. See
//! `docs/research/open-source.md` §3.15.

pub mod audio;
pub mod captions;
pub mod emergency;
pub mod error;
pub mod history;
pub mod incoming;
pub mod invitation;
pub mod media;
pub mod ngc_trouter;
pub mod relay;
pub mod signaling;
pub mod types;

pub use error::{Error, Result};
pub use types::*;
