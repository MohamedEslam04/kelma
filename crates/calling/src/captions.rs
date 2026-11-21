//! Live caption text: how it is framed on the media's data channel, what one message says, and
//! the few latest lines the caption strip shows.
//!
//! The media server sends captions on the SCTP data channel `main-channel` (the SDP's `x-data`
//! section), inside Teams' own framing: a header of at least 11 bytes (length, data id, sequence,
//! fragments left, source id, recipients), then a JSON payload. Captions are data id 3; their
//! JSON holds `recognitionResults` (or `textTracks`) entries with the speaker's `userId` and
//! `displayName`, the `text` and `isFinal`; a partial result is replaced by later ones of the same
//! utterance id.
//!
//! Facts: rusty-teams `crates/calling/src/data_channel.rs` and `captions.rs`
//! `parse_caption_message` (MIT). The media session opens the channel with
//! `MediaOptions::data_channel` (`media/sdp.rs` maps the `x-data` line); UNVERIFIED live by ironteams.

use serde_json::Value;
use std::collections::HashMap;

/// The data id of caption messages.
pub const CAPTIONS_DATA_ID: u8 = 3;
/// The data channel's label.
pub const DATA_CHANNEL_LABEL: &str = "main-channel";
const FIXED_HEADER: usize = 11;
const START_FLAG: u8 = 0x80;
const DATA_ID_MASK: u8 = 0x3f;
const LENGTH_HIGH_MASK: u8 = 0x0f;

/// One whole message of the data channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataMessage {
    pub data_id: u8,
    pub source_id: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, Default)]
struct Pending {
    source_id: u32,
    remaining: u8,
    next_sequence: u16,
    payload: Vec<u8>,
    started: bool,
}

// Adapted from rusty-teams (MIT), crates/calling/src/data_channel.rs `Depacketizer`.
/// Joins the fragments of each data id into whole messages.
#[derive(Debug, Default)]
pub struct Depacketizer {
    pending: HashMap<u8, Pending>,
}

impl Depacketizer {
    /// Takes one frame; a whole message once its last fragment is in. A frame out of sequence
    /// drops the message it belonged to.
    pub fn push(&mut self, frame: &[u8]) -> Option<DataMessage> {
        if frame.len() < FIXED_HEADER {
            return None;
        }
        let header = (usize::from(frame[0] & LENGTH_HIGH_MASK) << 8) | usize::from(frame[1]);
        if header < FIXED_HEADER || header > frame.len() {
            return None;
        }
        let data_id = frame[2] & DATA_ID_MASK;
        let sequence = u16::from_be_bytes([frame[3], frame[4]]);
        let remaining = frame[5];
        let source_id = u32::from_be_bytes([frame[6], frame[7], frame[8], frame[9]]);
        let p = self.pending.entry(data_id).or_default();
        if frame[2] & START_FLAG != 0 {
            *p = Pending {
                source_id,
                remaining,
                next_sequence: sequence,
                payload: Vec::new(),
                started: true,
            };
        } else if !p.started || p.next_sequence != sequence {
            *p = Pending::default();
            return None;
        } else {
            p.remaining = p.remaining.saturating_sub(1);
        }
        p.next_sequence = sequence.wrapping_add(1);
        p.payload.extend_from_slice(&frame[header..]);
        if p.remaining != 0 {
            return None;
        }
        let done = std::mem::take(p);
        Some(DataMessage {
            data_id,
            source_id: done.source_id,
            payload: done.payload,
        })
    }
}

/// One caption: who speaks and what they said so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionLine {
    /// The utterance; later results of the same utterance replace it.
    pub id: String,
    /// The speaker's MRI.
    pub user_id: String,
    /// The speaker's name as the service sent it (may be empty: look it up in the roster).
    pub speaker: String,
    pub text: String,
    pub is_final: bool,
}

/// The captions in one message (data id [`CAPTIONS_DATA_ID`]). Empty for anything else.
pub fn parse_caption_message(payload: &[u8]) -> Vec<CaptionLine> {
    let Ok(v) = serde_json::from_slice::<Value>(payload) else {
        return Vec::new();
    };
    ["recognitionResults", "textTracks"]
        .into_iter()
        .filter_map(|k| v[k].as_array())
        .flatten()
        .filter_map(line_from)
        .collect()
}

fn line_from(item: &Value) -> Option<CaptionLine> {
    let text = item["text"]
        .as_str()
        .or_else(|| item["rawText"].as_str())?
        .trim();
    if text.is_empty() {
        return None;
    }
    let user_id = item["userId"].as_str().unwrap_or_default().to_owned();
    let id = item["utteranceId"]
        .as_str()
        .or_else(|| item["id"].as_str())
        .map_or_else(|| user_id.clone(), str::to_owned);
    Some(CaptionLine {
        id,
        user_id,
        speaker: item["displayName"].as_str().unwrap_or_default().to_owned(),
        text: text.to_owned(),
        is_final: item["isFinal"].as_bool().unwrap_or(false),
    })
}

/// The latest caption lines, oldest first, one per utterance.
#[derive(Debug, Clone)]
pub struct Captions {
    keep: usize,
    lines: Vec<CaptionLine>,
}

impl Captions {
    pub fn new(keep: usize) -> Self {
        Captions {
            keep: keep.max(1),
            lines: Vec::new(),
        }
    }

    /// Adds a line, or updates its utterance in place. Returns whether anything changed.
    pub fn add(&mut self, line: CaptionLine) -> bool {
        if let Some(known) = self.lines.iter_mut().find(|l| l.id == line.id) {
            if *known == line {
                return false;
            }
            *known = line;
            return true;
        }
        self.lines.push(line);
        if self.lines.len() > self.keep {
            self.lines.remove(0);
        }
        true
    }

    pub fn lines(&self) -> &[CaptionLine] {
        &self.lines
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}
