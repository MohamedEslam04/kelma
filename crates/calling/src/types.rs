//! Types shared between the layers. Keep this file small: it is the contract the signalling,
//! media and audio modules are written against.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a call carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Modality {
    Audio,
    Video,
    ScreenSharing,
}

/// A participant as the NGC service names one (`8:orgid:<oid>`, `8:live:.cid.<hex>`, `28:<bot>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    pub mri: String,
    pub display_name: Option<String>,
    pub endpoint_id: Option<String>,
    pub participant_id: Option<String>,
}

/// An SDP as the service carries it: `{contentType, blob}`. `content_type` is
/// `application/sdp` (plain SDP, what mautrix-teams sends) or `application/sdp-ngc-1.0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdpBlob {
    pub content_type: String,
    pub blob: String,
}

/// Named callback URLs the service hands back (`acceptance`, `rejection`, `mediaAnswer`, `end`,
/// `leave`, `keepAlive`, `mute`, ...). Every later step is a POST to one of these.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Links(pub BTreeMap<String, String>);

impl Links {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

/// An incoming call as pushed on the `NextGenCalling` trouter registration.
#[derive(Debug, Clone, PartialEq)]
pub struct CallInvitation {
    pub call_id: String,
    pub from: Participant,
    pub modalities: Vec<Modality>,
    /// Links of the invitation itself (`acceptance`, `rejection`, `redirection`, `progress`, ...).
    pub links: Links,
    /// The caller's media offer, when the invitation carries one.
    pub offer: Option<SdpBlob>,
    /// The whole decoded push body, for fields not modelled yet.
    pub raw: serde_json::Value,
}

/// One TURN/STUN server for ICE, from `trap/tokens`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

/// Opus audio framing used between [`crate::audio`] and [`crate::media`]: 48 kHz, mono,
/// 20 ms frames (960 samples per frame).
pub const SAMPLE_RATE: u32 = 48_000;
pub const FRAME_SAMPLES: usize = 960;

/// One encoded Opus packet (20 ms).
pub type OpusPacket = Vec<u8>;
