//! Switching our video lines (screen share; the camera later) on and off with the service.
//!
//! - In a meeting the media server never renegotiates for it: the client flips lines it already
//!   negotiated with `UpdateMediaDescriptions` on the call leg's `updateMediaDescriptions` link
//!   (mautrix-teams `docs/openapi/teams-webclient.yaml` `/updateMediaDescriptions`, captured from
//!   the web client; mautrix-teams `video.go` `SetVideo`; rusty-teams `video_layout.rs`).
//! - In a 1:1 (direct) call the client sends a new offer, a `mediaNegotiation` on the other
//!   side's `mediaRenegotiation` link, with `ScreenSharer` among the `callModalities`
//!   (mautrix-teams `calls.go` `Renegotiate`; rusty-teams `hold.rs` `offer_modalities`).
//! - The media controller asks the sender for a size, a rate and keyframes on
//!   `call/controlVideoStreaming/` (mautrix-teams `video.go` `handleControl`).
//!
//! Facts only from mautrix-teams (AGPL); see `docs/protocol-calls.md` (Screen sharing).

use serde_json::{Value, json};

use super::{CallSession, NgcClient};
use crate::error::{Error, Result};
use crate::signaling::bodies;
use crate::types::{Links, Modality, SdpBlob};
use crate::video::VideoRole;

/// One line in an `UpdateMediaDescriptions`: its mid, its direction and, while it sends, its
/// label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDescription {
    pub mid: String,
    pub direction: &'static str,
    pub label: Option<&'static str>,
}

impl MediaDescription {
    /// The line `mid` that carries `role`: sending (its role's direction and label) or
    /// receive-only.
    pub fn line(mid: impl Into<String>, role: VideoRole, sending: bool) -> Self {
        MediaDescription {
            mid: mid.into(),
            direction: if sending {
                role.sending_direction()
            } else {
                "recvonly"
            },
            label: sending.then(|| role.label()),
        }
    }

    pub fn wire(&self) -> Value {
        let mut v = json!({"mid": self.mid, "direction": self.direction});
        if let Some(label) = self.label {
            v["label"] = json!(label);
        }
        v
    }
}

/// `<participantId>;ss_1` (share), `;v_2` (camera on) or `;v_1` (camera off).
pub fn negotiation_tag(s: &CallSession, role: VideoRole, sending: bool) -> String {
    format!(
        "{};{}",
        s.me.participant_id,
        role.negotiation_suffix(sending)
    )
}

/// `POST <updateMediaDescriptions>`. `request_id` grows by one per change.
pub fn update_media_descriptions_body(
    descriptions: &[MediaDescription],
    negotiation_tag: &str,
    request_id: u64,
) -> Value {
    json!({"UpdateMediaDescriptions": {"mediaDescriptions": {
        "descriptions": descriptions.iter().map(MediaDescription::wire).collect::<Vec<_>>(),
        "negotiationTag": negotiation_tag,
        "requestId": request_id,
    }}})
}

/// `callModalities` of a re-offer: audio, plus the modality of each line that sends.
pub fn video_modalities(sending: &[VideoRole]) -> Vec<Modality> {
    let mut m = vec![Modality::Audio];
    for role in sending {
        let modality = role.modality();
        if !m.contains(&modality) {
            m.push(modality);
        }
    }
    m
}

/// `POST <mediaRenegotiation>` (the other side's link): our new offer on a running direct call,
/// as the web client switches its share (mautrix-teams `Renegotiate`): `mediaNegotiation` with
/// our `mediaAnswer`/`rejection` links, `requiredFeatures: nonByPass`, the negotiation tag, and
/// `newOffer: false` (rusty-teams, a re-offer to the same peer).
pub fn video_renegotiation_body(
    s: &CallSession,
    modalities: &[Modality],
    offer: &SdpBlob,
    negotiation_tag: &str,
) -> Value {
    let mut body = bodies::negotiation_body(s, modalities, offer);
    let media = &mut body["mediaNegotiation"]["mediaContent"];
    media["newOffer"] = json!(false);
    media["negotiationTag"] = json!(negotiation_tag);
    body
}

/// What the media controller asks of one of our video sources (`controlVideoStreaming`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SenderControl {
    /// The `sourceId` the request is for (our line's `x-source-streamid`).
    pub source_id: u64,
    /// The raw `fmtParams` (`max-fs=…;max-fps=…;max-br=…;KeyFrame=1`).
    pub fmt_params: String,
}

impl SenderControl {
    pub fn keyframe(&self) -> bool {
        self.param("KeyFrame") == Some("1")
    }

    /// One `name=value` of `fmt_params`.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.fmt_params
            .split([';', ' ', ','])
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
    }
}

/// The requests in a `controlVideoStreaming` callback body.
pub fn parse_sender_controls(body: &Value) -> Vec<SenderControl> {
    let holder = body.get("controlVideoStreaming").unwrap_or(body);
    holder["controlInfo"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| SenderControl {
            source_id: c["sourceId"].as_u64().unwrap_or_default(),
            fmt_params: c["fmtParams"].as_str().unwrap_or_default().to_owned(),
        })
        .collect()
}

impl NgcClient {
    /// Flips our video lines in a meeting (see [`update_media_descriptions_body`]).
    pub async fn update_media_descriptions(
        &self,
        s: &CallSession,
        links: &Links,
        descriptions: &[MediaDescription],
        negotiation_tag: &str,
        request_id: u64,
    ) -> Result<()> {
        let url = links
            .get("updateMediaDescriptions")
            .ok_or_else(|| Error::Protocol("no 'updateMediaDescriptions' link".into()))?;
        let body = update_media_descriptions_body(descriptions, negotiation_tag, request_id);
        self.send(
            "update_media_descriptions",
            s,
            reqwest::Method::POST,
            url,
            Some(&body),
        )
        .await
        .map(drop)
    }

    /// Sends our re-offer on a direct call (see [`video_renegotiation_body`]). The answer comes
    /// inline or later on `call/mediaAnswer/`.
    pub async fn renegotiate_video(
        &self,
        s: &CallSession,
        links: &Links,
        modalities: &[Modality],
        offer: &SdpBlob,
        negotiation_tag: &str,
    ) -> Result<Option<SdpBlob>> {
        let url = links
            .get("mediaRenegotiation")
            .ok_or_else(|| Error::Protocol("no 'mediaRenegotiation' link".into()))?;
        let body = video_renegotiation_body(s, modalities, offer, negotiation_tag);
        let r = self
            .send(
                "media_renegotiation",
                s,
                reqwest::Method::POST,
                url,
                Some(&body),
            )
            .await?;
        Ok(bodies::parse_inline_answer(&r.body))
    }
}
