//! What the MCP server may touch: the accounts the user allowed (`mcp.json` in the config
//! directory, apart from the app's `config.json`) and the conversations, which are chats only.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use teams_proto::model::{ConversationKind, conversation_kind};

/// The allowlist file in the config directory.
pub const FILE: &str = "mcp.json";

/// The accounts (by label) the user allowed the MCP server to run on. Empty by default: chat
/// content the model reads leaves the machine, so every account is an explicit choice.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allowlist {
    #[serde(default)]
    pub allowed_accounts: Vec<String>,
}

impl Allowlist {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join(FILE)
    }

    /// The saved allowlist, or an empty one when there is no file yet.
    pub fn load(config_dir: &Path) -> Result<Allowlist> {
        let path = Self::path(config_dir);
        match std::fs::read_to_string(&path) {
            Ok(s) => {
                serde_json::from_str(&s).with_context(|| format!("reading {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Allowlist::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let path = Self::path(config_dir);
        std::fs::write(&path, serde_json::to_string_pretty(self)? + "\n")
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn allows(&self, label: &str) -> bool {
        self.allowed_accounts.iter().any(|a| a == label)
    }

    /// Adds `label`; false when it was already there.
    pub fn allow(&mut self, label: &str) -> bool {
        if self.allows(label) {
            return false;
        }
        self.allowed_accounts.push(label.to_string());
        self.allowed_accounts.sort();
        true
    }

    /// Removes `label`; false when it was not there.
    pub fn deny(&mut self, label: &str) -> bool {
        let before = self.allowed_accounts.len();
        self.allowed_accounts.retain(|a| a != label);
        self.allowed_accounts.len() != before
    }
}

/// A short name for a kind of conversation, as the tools print it.
pub fn kind_name(k: ConversationKind) -> &'static str {
    match k {
        ConversationKind::OneOnOne => "1:1 chat",
        ConversationKind::Group => "group chat",
        ConversationKind::Meeting => "meeting chat",
        ConversationKind::Notes => "your own notes",
        ConversationKind::Channel => "channel",
        ConversationKind::Other => "other",
    }
}

/// True for the kinds of conversation the tools work on.
pub fn is_chat(k: ConversationKind) -> bool {
    matches!(
        k,
        ConversationKind::OneOnOne
            | ConversationKind::Group
            | ConversationKind::Meeting
            | ConversationKind::Notes
    )
}

/// Checks that `id` names a chat the tools may read or write: a 1:1, group or meeting chat, or
/// the user's own notes. Team channels, channel threads (`…;messageid=…`) and anything
/// unrecognized are refused.
pub fn check_chat(id: &str) -> Result<ConversationKind, String> {
    if id.is_empty() || id.contains(';') || id.contains('/') {
        return Err(format!("{id:?} is not a chat id"));
    }
    let kind = conversation_kind(id, None);
    if is_chat(kind) {
        Ok(kind)
    } else if kind == ConversationKind::Channel {
        Err("IronTeams' MCP server works on chats only; team channels are off limits".into())
    } else {
        Err(format!("{id:?} is not a chat id"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ironteams-mcp-{name}-{}-{}",
            std::process::id(),
            teams_proto::time::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_allowlist_is_empty_until_an_account_is_allowed() {
        let dir = temp_dir("allow");
        let mut a = Allowlist::load(&dir).unwrap();
        assert!(!a.allows("default"));
        assert!(a.allow("work"));
        assert!(a.allow("default"));
        assert!(!a.allow("work"), "allowing twice changes nothing");
        a.save(&dir).unwrap();
        let back = Allowlist::load(&dir).unwrap();
        assert_eq!(back.allowed_accounts, ["default", "work"]);
        let mut back = back;
        assert!(back.deny("work"));
        assert!(!back.deny("work"));
        assert!(!back.allows("work"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_broken_allowlist_is_an_error_not_an_empty_list() {
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(Allowlist::path(&dir), "{not json").unwrap();
        assert!(Allowlist::load(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_chats_pass() {
        assert_eq!(check_chat("48:notes"), Ok(ConversationKind::Notes));
        assert_eq!(
            check_chat("19:aaa_bbb@unq.gbl.spaces"),
            Ok(ConversationKind::OneOnOne)
        );
        assert_eq!(check_chat("19:abc@thread.v2"), Ok(ConversationKind::Group));
        assert_eq!(
            check_chat("19:meeting_xyz@thread.v2"),
            Ok(ConversationKind::Meeting)
        );
        assert!(check_chat("19:abc@thread.tacv2").is_err(), "channel");
        assert!(check_chat("19:abc@thread.skype").is_err(), "old channel");
        assert!(
            check_chat("19:abc@thread.tacv2;messageid=1").is_err(),
            "thread"
        );
        assert!(check_chat("19:abc@thread.v2;messageid=1").is_err());
        assert!(check_chat("").is_err());
        assert!(check_chat("48:calllogs").is_err(), "not a chat");
    }
}
