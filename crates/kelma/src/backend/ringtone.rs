//! The ringtones Settings › Calls offers. The choice is local to kelma (Teams keeps its own per
//! device); the sounds themselves are generated in `call_direct`, which needs the calling stack.

use serde::{Deserialize, Serialize};

/// The sound of an incoming call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RingtoneChoice {
    /// The two-note chime.
    #[default]
    Chime,
    /// One long, gentle note.
    Soft,
    /// Three quick beeps.
    Pulse,
}

impl RingtoneChoice {
    pub const ALL: [RingtoneChoice; 3] = [
        RingtoneChoice::Chime,
        RingtoneChoice::Soft,
        RingtoneChoice::Pulse,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            RingtoneChoice::Chime => "chime",
            RingtoneChoice::Soft => "soft",
            RingtoneChoice::Pulse => "pulse",
        }
    }

    pub fn from_slug(s: &str) -> Option<RingtoneChoice> {
        Self::ALL.into_iter().find(|c| c.slug() == s)
    }
}
