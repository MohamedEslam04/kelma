//! Session persistence for kelma.
//!
//! - The refresh token (the only long-lived secret) lives in the system keyring via the
//!   Secret Service API (KWallet or GNOME Keyring). It is never written to disk by kelma.
//! - Non-secret settings (tenant, endpoint id) live in `config.json` in the config dir.
//! - Several accounts can be signed in. [`Config::accounts`] lists them and the legacy
//!   `account`/`tenant`/`epid`/`self_mri`/`display_name` fields always mirror the active one, so
//!   code (and older builds) that only knows one account keeps working. Each account has a
//!   keyring entry named by its label; a guest organization of the same account gets its own
//!   entry `label@tenant` once it issued a token.

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
    #[error("account: {0}")]
    Account(String),
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

/// Local calendar display settings (Calendar settings in the toolbar menu). Every field has a
/// default so config files written before it still load.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CalendarSettings {
    /// First day of the week: 0 for Sunday up to 6 for Saturday (Monday by default).
    pub first_day: u8,
    /// Work hours as minutes after midnight; `None` takes the mailbox's, then 09:00 to 17:00.
    pub work_start_min: Option<u32>,
    pub work_end_min: Option<u32>,
    /// Week and month views include Saturday and Sunday.
    pub show_weekends: bool,
    /// Show week numbers in the toolbar, the mini calendar and the month view.
    pub week_numbers: bool,
    /// A second time column in the day and week views, as a fixed UTC offset in minutes.
    pub second_tz_min: Option<i32>,
}

impl Default for CalendarSettings {
    fn default() -> Self {
        CalendarSettings {
            first_day: 1,
            work_start_min: None,
            work_end_min: None,
            show_weekends: true,
            week_numbers: false,
            second_tz_min: None,
        }
    }
}

/// A named calendar view: the view key (`workweek`, `week`, `month`, `day:N`) and time scale.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedView {
    pub name: String,
    pub view: String,
    pub scale: u32,
}

/// One organization (tenant) an account can work in: its home tenant or a guest tenant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgRef {
    pub tenant_id: String,
    pub name: String,
    #[serde(default)]
    pub is_home: bool,
}

impl From<&teams_proto::tenants::Tenant> for OrgRef {
    fn from(t: &teams_proto::tenants::Tenant) -> OrgRef {
        OrgRef {
            tenant_id: t.id.clone(),
            name: t.name.clone(),
            is_home: t.is_home,
        }
    }
}

/// A signed-in (or signed-out but remembered) account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    /// Keyring label and the name used on the command line.
    pub label: String,
    /// Sign-in name once known.
    #[serde(default)]
    pub upn: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub self_mri: Option<String>,
    /// The account's home tenant id once the org list was read.
    #[serde(default)]
    pub home_tenant: Option<String>,
    /// The tenant segment in use: `organizations`, the home tenant id or a guest tenant id.
    pub tenant: String,
    /// Endpoint id of this account (a UUID reused across runs).
    pub epid: String,
    /// Unix milliseconds of the last time this account was made active.
    #[serde(default)]
    pub last_used_ms: Option<i64>,
    /// The organizations last read from the Teams middle tier.
    #[serde(default)]
    pub orgs: Vec<OrgRef>,
}

impl Account {
    fn fresh(label: &str) -> Account {
        Account {
            label: label.into(),
            upn: None,
            display_name: None,
            self_mri: None,
            home_tenant: None,
            tenant: teams_proto::client_info::DEFAULT_TENANT.into(),
            epid: teams_proto::client::new_epid(),
            last_used_ms: None,
            orgs: Vec::new(),
        }
    }
}

/// Non-secret settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Keyring account label for the refresh token. This is the active account's label.
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
    /// The calendar view last used (`workweek`, `week`, `month` or `day:N`).
    #[serde(default)]
    pub calendar_view: Option<String>,
    /// The calendar time scale in minutes (15, 30 or 60).
    #[serde(default)]
    pub calendar_scale: Option<u32>,
    /// First day, work hours, weekends, week numbers and second time zone.
    #[serde(default)]
    pub calendar_settings: CalendarSettings,
    /// Views saved by name.
    #[serde(default)]
    pub calendar_saved_views: Vec<SavedView>,
    /// Composer: Enter sends the message (default). When false, Enter starts a new line and
    /// Ctrl+Enter sends.
    #[serde(default = "default_true")]
    pub enter_sends: bool,
    /// Emoji skin tone: 0 for the default yellow, 1..=5 for light to dark.
    #[serde(default)]
    pub skin_tone: u8,
    /// Recently used emoji, newest first.
    #[serde(default)]
    pub recent_emoji: Vec<String>,
    /// Every account. Empty in a file written before accounts existed; [`Config::normalize`]
    /// then adds the one the legacy fields describe.
    #[serde(default)]
    pub accounts: Vec<Account>,
}

fn default_true() -> bool {
    true
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
            calendar_view: None,
            calendar_scale: None,
            calendar_settings: CalendarSettings::default(),
            calendar_saved_views: Vec::new(),
            enter_sends: true,
            skin_tone: 0,
            recent_emoji: Vec::new(),
            accounts: Vec::new(),
        }
    }
}

fn now_ms() -> i64 {
    teams_proto::time::now_ms()
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Config {
    /// Makes `accounts` hold the active account, which a config of the one-account era lacks.
    /// Pure and in memory: nothing is written and the keyring is not touched.
    pub fn normalize(&mut self) {
        if self.account_by_label(&self.account).is_none() {
            self.accounts.push(Account {
                label: self.account.clone(),
                upn: None,
                display_name: self.display_name.clone(),
                self_mri: self.self_mri.clone(),
                home_tenant: None,
                tenant: self.tenant.clone(),
                epid: self.epid.clone(),
                last_used_ms: None,
                orgs: Vec::new(),
            });
        }
    }

    /// Copies the legacy fields (which callers edit directly) into the active account.
    pub fn sync_active(&mut self) {
        self.normalize();
        let (label, tenant, epid) = (self.account.clone(), self.tenant.clone(), self.epid.clone());
        let (mri, name) = (self.self_mri.clone(), self.display_name.clone());
        if let Some(a) = self.accounts.iter_mut().find(|a| a.label == label) {
            a.tenant = tenant;
            a.epid = epid;
            a.self_mri = mri;
            a.display_name = name;
        }
    }

    /// Sets the legacy fields from the account with this label, which must exist.
    fn load_active(&mut self, label: &str) {
        if let Some(a) = self.account_by_label(label).cloned() {
            self.account = a.label;
            self.tenant = a.tenant;
            self.epid = a.epid;
            self.self_mri = a.self_mri;
            self.display_name = a.display_name;
        }
    }

    pub fn account_by_label(&self, label: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.label == label)
    }

    /// The active account (the one `account` names).
    pub fn active_account(&self) -> Option<&Account> {
        self.account_by_label(&self.account)
    }

    /// Adds a signed-out account with its own endpoint id. It does not become active.
    pub fn add_account(&mut self, label: &str) -> Result<&Account> {
        self.sync_active();
        if !valid_label(label) {
            return Err(Error::Account(format!(
                "label {label:?} must be 1 to 64 letters, digits, '.', '_' or '-'"
            )));
        }
        if self.account_by_label(label).is_some() {
            return Err(Error::Account(format!("account {label:?} already exists")));
        }
        self.accounts.push(Account::fresh(label));
        Ok(self.accounts.last().expect("just pushed"))
    }

    /// Makes another account active; the legacy fields then hold its values.
    pub fn switch_account(&mut self, label: &str) -> Result<()> {
        self.sync_active();
        if self.account_by_label(label).is_none() {
            return Err(Error::Account(format!("no account {label:?}")));
        }
        if let Some(a) = self.accounts.iter_mut().find(|a| a.label == label) {
            a.last_used_ms = Some(now_ms());
        }
        self.load_active(label);
        Ok(())
    }

    /// Removes an account from the list (its keyring entries go with
    /// [`forget_account_tokens`]). Removing the active one activates the most recently used
    /// other account; removing the only one leaves a fresh signed-out `default`.
    pub fn remove_account(&mut self, label: &str) -> Result<()> {
        self.sync_active();
        if self.account_by_label(label).is_none() {
            return Err(Error::Account(format!("no account {label:?}")));
        }
        self.accounts.retain(|a| a.label != label);
        if self.accounts.is_empty() {
            self.accounts.push(Account::fresh("default"));
        }
        if self.account == label {
            let next = self
                .accounts
                .iter()
                .max_by_key(|a| a.last_used_ms.unwrap_or(0))
                .map(|a| a.label.clone())
                .expect("at least one account");
            self.load_active(&next);
        }
        Ok(())
    }

    /// Picks the tenant the active account works in (`organizations`, a home or a guest tenant
    /// id). Tokens for it are minted by the next sign-in or refresh.
    pub fn switch_tenant(&mut self, tenant: &str) -> Result<()> {
        if tenant.is_empty() {
            return Err(Error::Account("empty tenant".into()));
        }
        self.sync_active();
        self.tenant = tenant.into();
        self.sync_active();
        Ok(())
    }

    /// Remembers the organizations of the active account and its home tenant.
    pub fn set_orgs(&mut self, orgs: Vec<OrgRef>) {
        self.sync_active();
        let label = self.account.clone();
        if let Some(a) = self.accounts.iter_mut().find(|a| a.label == label) {
            if let Some(h) = orgs.iter().find(|o| o.is_home) {
                a.home_tenant = Some(h.tenant_id.clone());
            }
            a.orgs = orgs;
        }
    }

    fn home_tenant(&self) -> Option<&str> {
        self.active_account().and_then(|a| a.home_tenant.as_deref())
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
            Ok(s) => {
                let mut c: Config = serde_json::from_str(&s)?;
                c.normalize();
                Ok(c)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut c = Config::default();
                c.normalize();
                self.save_config(&c)?;
                Ok(c)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Saves the config atomically, after copying the legacy fields into the active account.
    pub fn save_config(&self, c: &Config) -> Result<()> {
        let mut c = c.clone();
        c.sync_active();
        std::fs::create_dir_all(&self.config_dir)?;
        let tmp = self.config_path().with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&c)?)?;
        std::fs::rename(tmp, self.config_path())?;
        Ok(())
    }
}

/// Where refresh tokens are kept. The system keyring in the app; an in-memory map in tests.
pub trait TokenStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, token: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

/// The Secret Service keyring (KWallet or GNOME Keyring).
pub struct SystemKeyring;

fn entry(key: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, key)?)
}

impl TokenStore for SystemKeyring {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match entry(key)?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn set(&self, key: &str, token: &str) -> Result<()> {
        entry(key)?.set_password(token)?;
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// A token store in memory, for tests.
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<std::collections::BTreeMap<String, String>>);

impl MemoryStore {
    /// The stored keys, sorted.
    pub fn keys(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
}

impl TokenStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, token: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), token.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// The keyring key of an account's token for a tenant. The home tenant (the sign-in segment
/// `organizations`, or the home tenant id when known) uses the plain label, as before accounts
/// existed; a guest tenant uses `label@tenant`. Labels cannot contain `@`.
pub fn token_key(label: &str, tenant: &str, home_tenant: Option<&str>) -> String {
    if tenant == teams_proto::client_info::DEFAULT_TENANT || Some(tenant) == home_tenant {
        label.to_string()
    } else {
        format!("{label}@{tenant}")
    }
}

/// Stores a refresh token for an account in a tenant.
pub fn save_token_in(
    store: &dyn TokenStore,
    label: &str,
    tenant: &str,
    home_tenant: Option<&str>,
    token: &str,
) -> Result<()> {
    store.set(&token_key(label, tenant, home_tenant), token)
}

/// Removes every refresh token of an account: the home one and each guest tenant it knows.
pub fn forget_account_tokens(store: &dyn TokenStore, account: &Account) -> Result<()> {
    store.delete(&account.label)?;
    let tenants = account
        .orgs
        .iter()
        .map(|o| o.tenant_id.as_str())
        .chain(std::iter::once(account.tenant.as_str()));
    for t in tenants {
        store.delete(&token_key(
            &account.label,
            t,
            account.home_tenant.as_deref(),
        ))?;
    }
    Ok(())
}

/// Stores the refresh token in the system keyring.
pub fn save_refresh_token(account: &str, token: &str) -> Result<()> {
    SystemKeyring.set(account, token)
}

/// Loads the refresh token, or `None` if the user has not signed in.
pub fn load_refresh_token(account: &str) -> Result<Option<String>> {
    SystemKeyring.get(account)
}

/// Removes the refresh token (sign out).
pub fn delete_refresh_token(account: &str) -> Result<()> {
    SystemKeyring.delete(account)
}

/// Builds a protocol session from config plus a token store, if signed in. A guest tenant
/// without a token of its own starts from the account's home token, which the first refresh
/// redeems at the guest tenant.
pub fn stored_session_in(
    store: &dyn TokenStore,
    c: &Config,
) -> Result<Option<teams_proto::Session>> {
    let home = c.home_tenant();
    let own = token_key(&c.account, &c.tenant, home);
    let token = match store.get(&own)? {
        Some(t) => Some(t),
        None if own != c.account => store.get(&c.account)?,
        None => None,
    };
    Ok(token.map(|rt| teams_proto::Session {
        tenant: c.tenant.clone(),
        refresh_token: rt,
        epid: c.epid.clone(),
    }))
}

/// Builds a protocol session from config plus keyring, if signed in.
pub fn stored_session(c: &Config) -> Result<Option<teams_proto::Session>> {
    stored_session_in(&SystemKeyring, c)
}

/// Persists the (possibly rotated) refresh token of a live client, under the tenant the
/// client works in.
pub async fn persist_in(
    store: &dyn TokenStore,
    c: &Config,
    client: &teams_proto::Client,
) -> Result<()> {
    let s = client.session().await;
    save_token_in(
        store,
        &c.account,
        &s.tenant,
        c.home_tenant(),
        &s.refresh_token,
    )
}

/// Persists the (possibly rotated) refresh token of a live client.
pub async fn persist(c: &Config, client: &teams_proto::Client) -> Result<()> {
    persist_in(&SystemKeyring, c, client).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- accounts and orgs (X2.1) ----

    const OLD: &str = r#"{"account":"default","tenant":"organizations","epid":"e1","self_mri":"8:orgid:me","display_name":"Me"}"#;

    fn old_config() -> Config {
        let mut c: Config = serde_json::from_str(OLD).unwrap();
        c.normalize();
        c
    }

    #[test]
    fn old_config_migrates_to_one_account_with_the_old_label() {
        let c = old_config();
        assert_eq!(c.accounts.len(), 1);
        let a = &c.accounts[0];
        assert_eq!(a.label, "default");
        assert_eq!(a.epid, "e1");
        assert_eq!(a.tenant, "organizations");
        assert_eq!(a.self_mri.as_deref(), Some("8:orgid:me"));
        assert_eq!(a.display_name.as_deref(), Some("Me"));
        // The legacy fields still hold the active account's values.
        assert_eq!((c.account.as_str(), c.epid.as_str()), ("default", "e1"));
    }

    #[test]
    fn migrated_account_reads_the_old_keyring_entry_without_writes() {
        let store = MemoryStore::default();
        store.set("default", "rt-old").unwrap();
        let c = old_config();
        let s = stored_session_in(&store, &c).unwrap().unwrap();
        assert_eq!(s.refresh_token, "rt-old");
        assert_eq!(s.tenant, "organizations");
        assert_eq!(s.epid, "e1");
        assert_eq!(store.keys(), vec!["default".to_string()], "nothing written");
    }

    #[test]
    fn load_config_of_an_old_file_migrates_in_memory_and_leaves_the_file_alone() {
        let dir = std::env::temp_dir().join(format!("kelma-session-mig-{}", std::process::id()));
        let p = Paths {
            config_dir: dir.join("config"),
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
        };
        std::fs::create_dir_all(&p.config_dir).unwrap();
        std::fs::write(p.config_dir.join("config.json"), OLD).unwrap();
        let c = p.load_config().unwrap();
        assert_eq!(c.accounts.len(), 1);
        assert_eq!(
            std::fs::read_to_string(p.config_dir.join("config.json")).unwrap(),
            OLD
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saved_config_keeps_the_legacy_fields_so_an_older_build_still_reads_it() {
        let mut c = old_config();
        c.add_account("work").unwrap();
        c.switch_account("work").unwrap();
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["account"], "work");
        assert_eq!(json["tenant"], "organizations");
        assert!(json["epid"].as_str().unwrap().len() == 36);
        assert_eq!(json["accounts"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn add_switch_remove_accounts() {
        let mut c = old_config();
        c.add_account("work").unwrap();
        assert!(c.add_account("work").is_err(), "duplicate label");
        assert!(c.add_account("").is_err(), "empty label");
        assert!(c.add_account("a@b").is_err(), "label must not contain @");
        assert_eq!(c.account, "default", "adding does not switch");
        let work_epid = c.account_by_label("work").unwrap().epid.clone();
        assert_ne!(work_epid, "e1", "per-account endpoint id");

        c.switch_account("work").unwrap();
        assert_eq!(
            (c.account.as_str(), c.epid.as_str()),
            ("work", work_epid.as_str())
        );
        assert_eq!(c.self_mri, None, "the new account has no identity yet");
        assert!(c.account_by_label("work").unwrap().last_used_ms.is_some());
        assert!(c.switch_account("nope").is_err());

        // Edits through the legacy fields land on the active account.
        c.self_mri = Some("8:orgid:w".into());
        c.sync_active();
        c.switch_account("default").unwrap();
        assert_eq!(c.self_mri.as_deref(), Some("8:orgid:me"));
        assert_eq!(
            c.account_by_label("work").unwrap().self_mri.as_deref(),
            Some("8:orgid:w")
        );

        c.remove_account("default").unwrap();
        assert_eq!(
            c.account, "work",
            "removing the active account switches to another"
        );
        assert_eq!(c.accounts.len(), 1);
        c.remove_account("work").unwrap();
        assert_eq!(
            c.accounts.len(),
            1,
            "the last account is replaced by a fresh signed-out one"
        );
        assert_eq!(c.account, "default");
        assert_ne!(c.epid, work_epid);
        assert!(c.remove_account("ghost").is_err());
    }

    #[test]
    fn save_config_syncs_legacy_field_edits_into_the_active_account() {
        let dir = std::env::temp_dir().join(format!("kelma-session-sync-{}", std::process::id()));
        let p = Paths {
            config_dir: dir.join("config"),
            data_dir: dir.join("data"),
            cache_dir: dir.join("cache"),
        };
        let mut c = p.load_config().unwrap();
        c.display_name = Some("Someone".into());
        p.save_config(&c).unwrap();
        let back = p.load_config().unwrap();
        assert_eq!(back.accounts[0].display_name.as_deref(), Some("Someone"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tokens_are_per_account_and_per_tenant() {
        let store = MemoryStore::default();
        let mut c = old_config();
        c.add_account("work").unwrap();
        store.set("default", "rt-default").unwrap();
        store.set("work", "rt-work").unwrap();
        c.switch_account("work").unwrap();
        assert_eq!(
            stored_session_in(&store, &c)
                .unwrap()
                .unwrap()
                .refresh_token,
            "rt-work"
        );

        // A guest tenant without its own token re-mints from the home token.
        c.switch_tenant("guest-1").unwrap();
        assert_eq!(c.tenant, "guest-1");
        let s = stored_session_in(&store, &c).unwrap().unwrap();
        assert_eq!(
            (s.tenant.as_str(), s.refresh_token.as_str()),
            ("guest-1", "rt-work")
        );

        // Once the guest tenant issued its own token, that one wins and the home one stays.
        store.set("work@guest-1", "rt-work-guest").unwrap();
        let s = stored_session_in(&store, &c).unwrap().unwrap();
        assert_eq!(s.refresh_token, "rt-work-guest");
        assert_eq!(store.get("work").unwrap().as_deref(), Some("rt-work"));
        assert_eq!(token_key("work", "guest-1", None), "work@guest-1");
        assert_eq!(token_key("work", "organizations", None), "work");
        assert_eq!(token_key("work", "home-id", Some("home-id")), "work");

        // Back to the home tenant.
        c.switch_tenant("organizations").unwrap();
        assert_eq!(
            stored_session_in(&store, &c)
                .unwrap()
                .unwrap()
                .refresh_token,
            "rt-work"
        );
    }

    #[test]
    fn tenant_choice_is_remembered_per_account() {
        let mut c = old_config();
        c.add_account("work").unwrap();
        c.switch_account("work").unwrap();
        c.switch_tenant("guest-1").unwrap();
        c.switch_account("default").unwrap();
        assert_eq!(c.tenant, "organizations");
        c.switch_account("work").unwrap();
        assert_eq!(c.tenant, "guest-1");
    }

    #[test]
    fn org_list_is_cached_and_names_the_home_tenant() {
        let mut c = old_config();
        c.set_orgs(vec![
            OrgRef {
                tenant_id: "home-id".into(),
                name: "Home".into(),
                is_home: true,
            },
            OrgRef {
                tenant_id: "guest-1".into(),
                name: "Guest".into(),
                is_home: false,
            },
        ]);
        let a = c.active_account().unwrap();
        assert_eq!(a.home_tenant.as_deref(), Some("home-id"));
        assert_eq!(a.orgs.len(), 2);
        // The home tenant id maps to the plain account key.
        let store = MemoryStore::default();
        store.set("default", "rt").unwrap();
        c.switch_tenant("home-id").unwrap();
        assert_eq!(
            stored_session_in(&store, &c)
                .unwrap()
                .unwrap()
                .refresh_token,
            "rt"
        );
    }

    #[test]
    fn org_ref_comes_from_a_parsed_tenant() {
        let t = teams_proto::tenants::parse_tenants(&serde_json::json!([
            {"tenantId":"t1","tenantName":"Home","tenantRelationship":"Default"}
        ]));
        let o = OrgRef::from(&t[0]);
        assert_eq!(
            (o.tenant_id.as_str(), o.name.as_str(), o.is_home),
            ("t1", "Home", true)
        );
    }

    #[test]
    fn persist_and_forget_use_the_matching_keys() {
        let store = MemoryStore::default();
        let mut c = old_config();
        c.set_orgs(vec![OrgRef {
            tenant_id: "guest-1".into(),
            name: "G".into(),
            is_home: false,
        }]);
        save_token_in(
            &store,
            &c.account,
            "guest-1",
            c.active_account().and_then(|a| a.home_tenant.as_deref()),
            "x",
        )
        .unwrap();
        store.set("default", "h").unwrap();
        assert_eq!(
            store.keys(),
            vec!["default".to_string(), "default@guest-1".to_string()]
        );
        forget_account_tokens(&store, c.active_account().unwrap()).unwrap();
        assert!(
            store.keys().is_empty(),
            "sign-out removes the home and every guest token"
        );
    }

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

    #[test]
    fn config_without_calendar_fields_still_loads() {
        let old = r#"{"account":"default","tenant":"organizations","epid":"e"}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        assert_eq!((c.calendar_view, c.calendar_scale), (None, None));
        let c = Config {
            calendar_view: Some("day:3".into()),
            calendar_scale: Some(15),
            ..c
        };
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn calendar_settings_and_saved_views_default_for_old_files_and_roundtrip() {
        let old = r#"{"account":"default","tenant":"organizations","epid":"e"}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        assert_eq!(c.calendar_settings, CalendarSettings::default());
        assert_eq!(c.calendar_settings.first_day, 1, "Monday");
        assert!(c.calendar_settings.show_weekends);
        assert!(!c.calendar_settings.week_numbers);
        assert!(c.calendar_saved_views.is_empty());

        // A partial settings object keeps the defaults of what it leaves out.
        let c: Config = serde_json::from_str(&old.replace(
            '}',
            r#","calendar_settings":{"first_day":0,"second_tz_min":-300}}"#,
        ))
        .unwrap();
        assert_eq!(c.calendar_settings.first_day, 0);
        assert_eq!(c.calendar_settings.second_tz_min, Some(-300));
        assert!(c.calendar_settings.show_weekends);

        let mut c = c;
        c.calendar_saved_views.push(SavedView {
            name: "Planning".into(),
            view: "day:3".into(),
            scale: 15,
        });
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn config_written_before_the_composer_settings_still_loads() {
        let old = r#"{"account":"default","tenant":"organizations","epid":"e"}"#;
        let c: Config = serde_json::from_str(old).unwrap();
        assert!(c.enter_sends, "Enter sends by default");
        assert_eq!(c.skin_tone, 0);
        assert!(c.recent_emoji.is_empty());
        let mut c = c;
        c.enter_sends = false;
        c.skin_tone = 3;
        c.recent_emoji = vec!["👍".into()];
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
