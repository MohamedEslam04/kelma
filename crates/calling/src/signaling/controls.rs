//! In-meeting controls of our own endpoint: self mute/unmute and raise/lower hand.
//!
//! - **Mute** as the service sees it is our endpoint state: `POST updateEndpointState` with
//!   `endpointState.state.isMuted` and a rising `endpointStateSequenceNumber`; the change shows in
//!   everyone's roster as our endpoint's `endpointState`. It does not stop the audio: the caller
//!   also stops sending microphone packets while muted (rusty-teams disables the track and posts
//!   the state, `call/audio_switch.rs` `apply_mute`). Facts: rusty-teams `signaling.rs`
//!   `update_endpoint_state` (MIT), mautrix-teams `calls.go` `updateEndpointState` and
//!   `teams-webclient.yaml` `/updateEndpointState` (AGPL, facts only).
//! - **Hand** is a published state: `POST publishState` with a user-level `raiseHands` state,
//!   answered with its `stateId` and queue rank; `POST removeState` with that id lowers it. Facts:
//!   rusty-teams `signaling.rs` `raise_hand`/`lower_hands` (MIT), mautrix-teams `calls.go`
//!   `SetHandRaised` (facts only).
//!
//! Each operation goes to the conversation's link of the same name, else to
//! `<controller>/<name>` (mautrix-teams addresses them that way).
//!
//! Server-side mute by an organizer shows as `serverMuted` on a roster stream (see
//! [`super::roster`]); the organizer operations (`mute` link) are not modelled here.

use super::{CallSession, insert_path};
use crate::types::Links;
use serde_json::{Value, json};

const RAISE_HANDS: &str = "raiseHands";

/// `POST updateEndpointState`: publish whether we can be heard.
pub fn update_endpoint_state_body(s: &CallSession, sequence: u64, muted: bool) -> Value {
    json!({
        "from": s.me.wire(),
        "endpointState": {
            "endpointStateSequenceNumber": sequence,
            "endpointProperties": {
                "additionalEndpointProperties": {"infoShownInReportMode": "FullInformation"}
            },
            "state": {"isMuted": muted},
        },
    })
}

/// `POST publishState`: raise our hand.
pub fn raise_hand_body(s: &CallSession, sequence: u64) -> Value {
    json!({
        "from": s.me.wire(),
        "publishedState": {
            "stateType": RAISE_HANDS,
            "level": "user",
            "content": {"skinTone": 0},
            "sequenceNumber": sequence,
        },
    })
}

/// `POST removeState`: withdraw the named published states (a raised hand).
pub fn remove_state_body(s: &CallSession, sequence: u64, state_ids: &[String]) -> Value {
    json!({
        "from": s.me.wire(),
        "sequenceNumber": sequence,
        "scope": "specified",
        "stateIds": state_ids,
    })
}

/// What `publishState` answered for a raised hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandReceipt {
    pub state_id: Option<String>,
    /// Place in the hand queue (1 = first).
    pub rank: Option<u64>,
}

pub fn parse_hand_receipt(body: &Value) -> HandReceipt {
    let r = &body["publishStateResponse"];
    HandReceipt {
        state_id: r["stateId"].as_str().map(str::to_owned),
        rank: r["typeRank"].as_u64(),
    }
}

/// Where the conversation operation `name` goes: its link, else `<controller>/<name>`.
pub fn control_url(links: &Links, controller: &str, name: &str) -> String {
    links
        .get(name)
        .map(str::to_owned)
        .unwrap_or_else(|| insert_path(controller, &format!("/{name}")))
}

/// Our side of the controls in one call: the sequence numbers the service expects to rise, the
/// mute state we last published and the hand we raised. Builds each request; the network side is
/// [`super::NgcClient::set_muted`] and friends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Controls {
    endpoint_sequence: u64,
    state_sequence: u64,
    muted: bool,
    hand: Option<String>,
}

impl Controls {
    pub fn muted(&self) -> bool {
        self.muted
    }

    /// The `stateId` of the hand we raised, while it is up.
    pub fn hand_state_id(&self) -> Option<&str> {
        self.hand.as_deref()
    }

    /// The `updateEndpointState` request that sets our mute state, which it remembers.
    pub fn mute_request(
        &mut self,
        s: &CallSession,
        links: &Links,
        controller: &str,
        muted: bool,
    ) -> (String, Value) {
        self.muted = muted;
        self.repeat_mute_request(s, links, controller)
    }

    /// The current mute state again. mautrix-teams sends it after joining and after the lobby
    /// admits us: Teams ignored the first update after either.
    pub fn repeat_mute_request(
        &mut self,
        s: &CallSession,
        links: &Links,
        controller: &str,
    ) -> (String, Value) {
        self.endpoint_sequence += 1;
        (
            control_url(links, controller, "updateEndpointState"),
            update_endpoint_state_body(s, self.endpoint_sequence, self.muted),
        )
    }

    /// The `publishState` request that raises our hand. Record the answer with
    /// [`Self::hand_raised`].
    pub fn raise_hand_request(
        &mut self,
        s: &CallSession,
        links: &Links,
        controller: &str,
    ) -> (String, Value) {
        self.state_sequence += 1;
        (
            control_url(links, controller, "publishState"),
            raise_hand_body(s, self.state_sequence),
        )
    }

    pub fn hand_raised(&mut self, receipt: &HandReceipt) {
        self.hand = receipt.state_id.clone();
    }

    /// The `removeState` request that lowers our hand: the one we raised, else the one the
    /// roster shows for us (raised elsewhere). `None` when there is no hand to lower.
    pub fn lower_hand_request(
        &mut self,
        s: &CallSession,
        links: &Links,
        controller: &str,
        roster_hand: Option<&str>,
    ) -> Option<(String, Value)> {
        let id = self
            .hand
            .take()
            .or_else(|| roster_hand.map(str::to_owned))?;
        self.state_sequence += 1;
        Some((
            control_url(links, controller, "removeState"),
            remove_state_body(s, self.state_sequence, &[id]),
        ))
    }
}
