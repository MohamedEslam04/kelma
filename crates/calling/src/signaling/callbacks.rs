//! Callback URLs we publish on our calling trouter's `surl`.
//!
//! Every link the service may later call is a URL the client builds itself:
//! `<surl>callAgent/<sessionId>/<hash8>/<call|conversation>/<event>/`. Trouter routes anything
//! under the surl to our socket, so these need no registration.

use serde_json::{Map, Value};

/// Links the web client publishes when it accepts a call (also sent back in the trouter reply to
/// a `call/acceptance` callback on a call we placed).
pub const ACCEPTANCE_LINK_NAMES: [&str; 7] = [
    "mediaRenegotiation",
    "transfer",
    "replacement",
    "balanceUpdate",
    "retargetCompletion",
    "controlVideoStreaming",
    "updateMediaDescriptions",
];

/// The callback URLs of one call session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallbackLinks {
    base: String,
    session_id: String,
}

// Adapted from eisbaw/ost (MIT), src/calling/signaling.rs `trouter_callback`.
/// FNV-1a (32 bit) over `parts`, as the web client tags each callback path.
fn fnv1a32(parts: &[&str]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in parts.iter().flat_map(|p| p.bytes()) {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl CallbackLinks {
    pub fn new(surl: &str, session_id: &str) -> Self {
        let mut base = surl.to_owned();
        if !base.ends_with('/') {
            base.push('/');
        }
        CallbackLinks {
            base,
            session_id: session_id.to_owned(),
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The link for `path` (`call/acceptance/`, `conversation/rosterUpdate/`, ...).
    pub fn url(&self, path: &str) -> String {
        let tag = fnv1a32(&[&self.session_id, path]);
        format!(
            "{}callAgent/{}/{tag:08x}/{path}",
            self.base, self.session_id
        )
    }

    /// `<...>/call/<event>/`.
    pub fn call(&self, event: &str) -> String {
        self.url(&format!("call/{event}/"))
    }

    /// `<...>/conversation/<event>/`.
    pub fn conversation(&self, event: &str) -> String {
        self.url(&format!("conversation/{event}/"))
    }

    /// `{name: <...>/call/<name>/}` for each name.
    pub fn call_links(&self, names: &[&str]) -> Value {
        Value::Object(
            names
                .iter()
                .map(|n| ((*n).to_owned(), Value::String(self.call(n))))
                .collect::<Map<_, _>>(),
        )
    }

    /// `{name: <...>/conversation/<name>/}` for each name.
    pub fn conversation_links(&self, names: &[&str]) -> Value {
        Value::Object(
            names
                .iter()
                .map(|n| ((*n).to_owned(), Value::String(self.conversation(n))))
                .collect::<Map<_, _>>(),
        )
    }

    /// The links published with a call acceptance.
    pub fn acceptance_links(&self) -> Value {
        self.call_links(&ACCEPTANCE_LINK_NAMES)
    }
}
