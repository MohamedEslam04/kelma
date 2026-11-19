// STUB: replaced by feat/calls-ui
//! The call manager and the in-call window live on feat/calls-ui; this stub only carries the
//! interface the 1:1 call flows (`call_direct`) are written against, so they compile here.

// The stub is only partly used on this branch.
#![allow(dead_code)]

use calling::media::{MediaEvents, MediaSession};
use calling::ngc_trouter::CallPush;
use calling::signaling::{CallSession, NgcClient};
use calling::{CallInvitation, Links};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// What a call the manager runs is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallKind {
    Meeting { title: String },
    Direct { peer_name: String },
}

/// Owns the calling trouter and every call in progress (feat/calls-ui).
#[derive(Default)]
pub struct CallManager;

impl CallManager {
    /// Takes over a connected call: its callbacks, audio and the in-call window.
    pub fn adopt(
        &self,
        session: CallSession,
        links: Links,
        media: MediaSession,
        events: MediaEvents,
        kind: CallKind,
    ) -> anyhow::Result<()> {
        let _ = (session, links, media, events, kind);
        anyhow::bail!("the in-call window is not on this branch")
    }

    /// Incoming call invitations from the calling trouter.
    pub fn subscribe_invitations(&self) -> UnboundedReceiver<Box<CallInvitation>> {
        unbounded_channel().1
    }

    /// Calling pushes that belong to no adopted call (ringing-phase callbacks).
    pub fn subscribe_unmatched(&self) -> UnboundedReceiver<CallPush> {
        unbounded_channel().1
    }

    /// The calling trouter's surl, `None` while it is not connected.
    pub fn surl(&self) -> Option<String> {
        None
    }

    /// The conversation service client.
    pub async fn ngc(&self) -> anyhow::Result<NgcClient> {
        anyhow::bail!("no calling endpoints on this branch")
    }
}

/// The running manager, once kelma is signed in.
pub fn manager() -> Option<std::sync::Arc<CallManager>> {
    None
}
