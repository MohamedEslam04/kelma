//! Session persistence for kelma.
//!
//! - The refresh token (the only long-lived secret) lives in the system keyring via the
//!   Secret Service API (KWallet or GNOME Keyring). It is never written to disk by kelma.
//! - Non-secret settings (tenant, endpoint id) live in `config.json` in the config dir.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SERVICE: &str = "kelma";

/// Errors from session persistence.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("keyring: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no home directory")]
    NoHome,
}

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Where a chat opens ("When I open a chat, take me to" in Teams settings).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpenChatAt {
    /// The first unread message (at the "New messages" divider); the newest when all is read.
    #[default]
    FirstUnread,
    /// Always the newest message.
    Newest,
}

/// Non-secret settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Keyring account label for the refresh token.
    pub account: String,
    /// Tenant segment for sign-in (`organizations` or a tenant id).
    pub tenant: String,
    /// Persistent endpoint id.
    pub epid: String,
    /// Our MRI once known.
    #[serde(default)]
    pub self_mri: Option<String>,
    /// Our display name once known.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Where a chat opens. Missing in older config files.
    #[serde(default)]
    pub open_chat_at: OpenChatAt,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            account: "default".into(),
            tenant: teams_proto::client_info::DEFAULT_TENANT.into(),
            epid: teams_proto::client::new_epid(),
            self_mri: None,
            display_name: None,
            open_chat_at: OpenChatAt::default(),
        }
    }
}

/// Standard directories for kelma.
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    /// XDG directories (`~/.config/kelma`, `~/.local/share/kelma`, `~/.cache/kelma`).
    /// `KELMA_HOME` overrides all three (used by tests).
    pub fn new() -> Result<Paths> {
        if let Ok(home) = std::env::var("KELMA_HOME") {
            let h = PathBuf::from(home);
            return Ok(Paths {
                config_dir: h.join("config"),
                data_dir: h.join("data"),
                cache_dir: h.join("cache"),
            });
        }
        let d = directories::ProjectDirs::from("", "", SERVICE).ok_or(Error::NoHome)?;
        Ok(Paths {
            config_dir: d.config_dir().to_path_buf(),
            data_dir: d.data_dir().to_path_buf(),
            cache_dir: d.cache_dir().to_path_buf(),
        })
    }

    /// SQLite cache path.
    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("cache.sqlite")
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir.join("config.json")
    }

    /// Loads the config, creating a default one (with a fresh epid) if none exists.
    pub fn load_config(&self) -> Result<Config> {
        match std::fs::read_to_string(self.config_path()) {
            Ok(s) => Ok(serde_json::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let c = Config::default();
                self.save_config(&c)?;
                Ok(c)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Saves the config atomically.
    pub fn save_config(&self, c: &Config) -> Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        let tmp = self.config_path().with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(c)?)?;
        std::fs::rename(tmp, self.config_path())?;
        Ok(())
    }
}

fn entry(account: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, account)?)
}

/// Stores the refresh token in the system keyring.
pub fn save_refresh_token(account: &str, token: &str) -> Result<()> {
    entry(account)?.set_password(token)?;
    Ok(())
}

/// Loads the refresh token, or `None` if the user has not signed in.
pub fn load_refresh_token(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Removes the refresh token (sign out).
pub fn delete_refresh_token(account: &str) -> Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Builds a protocol session from config plus keyring, if signed in.
pub fn stored_session(c: &Config) -> Result<Option<teams_proto::Session>> {
    Ok(
        load_refresh_token(&c.account)?.map(|rt| teams_proto::Session {
            tenant: c.tenant.clone(),
            refresh_token: rt,
            epid: c.epid.clone(),
        }),
    )
}

/// Persists the (possibly rotated) refresh token of a live client.
pub async fn persist(c: &Config, client: &teams_proto::Client) -> Result<()> {
    save_refresh_token(&c.account, &client.session().await.refresh_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_and_default_epid() {
        let dir = std::env::temp_dir().join(format!("kelma-session-test-{}", std::process::id()));
        let p = Paths {
            config_dir: dir.join("config"),
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
        };
        let c = p.load_config().unwrap();
        assert_eq!(c.tenant, "organizations");
        assert_eq!(c.epid.len(), 36);
        let again = p.load_config().unwrap();
        assert_eq!(c, again, "epid must persist across loads");
        let raw = std::fs::read_to_string(dir.join("config/config.json")).unwrap();
        assert!(!raw.contains("refresh"), "config must not contain tokens");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn open_chat_at_defaults_for_old_config_files_and_roundtrips() {
        let old = r#"{"account":"default","tenant":"organizations","epid":"e"}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        assert_eq!(c.open_chat_at, OpenChatAt::FirstUnread);

        let c: Config =
            serde_json::from_str(&old.replace('}', r#","open_chat_at":"newest"}"#)).unwrap();
        assert_eq!(c.open_chat_at, OpenChatAt::Newest);
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
