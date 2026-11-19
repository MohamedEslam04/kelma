//! An app's definition (its manifest as the app service serves it) and what it works in.

use serde_json::Value;
use teams_proto::graph::str_of;

/// Where an app can be added or used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorksIn {
    pub personal: bool,
    pub team: bool,
    pub chat: bool,
    pub meeting: bool,
}

/// A device permission an app's manifest asks for. The Store settings (gear) grant or deny
/// each one per app; the grants live in kelma's own config (see [`crate::Grants`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DevicePermission {
    Media,
    Geolocation,
    Notifications,
    Midi,
    OpenExternal,
}

impl DevicePermission {
    pub const ALL: [DevicePermission; 5] = [
        DevicePermission::Media,
        DevicePermission::Geolocation,
        DevicePermission::Notifications,
        DevicePermission::Midi,
        DevicePermission::OpenExternal,
    ];

    /// The manifest spelling (the service sends both `media` and `Media`).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str().eq_ignore_ascii_case(s))
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DevicePermission::Media => "media",
            DevicePermission::Geolocation => "geolocation",
            DevicePermission::Notifications => "notifications",
            DevicePermission::Midi => "midi",
            DevicePermission::OpenExternal => "openExternal",
        }
    }
}

/// A personal tab of an app: what opens when the app is opened from the app bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticTab {
    pub entity_id: String,
    pub name: String,
    /// The tab URL with `{placeholders}` (`{locale}`, `{tid}`, …) for the web host to fill.
    pub content_url: Option<String>,
    pub website_url: Option<String>,
}

/// An app definition: everything the app bar, the flyout and the app page show.
#[derive(Debug, Clone, PartialEq)]
pub struct AppDefinition {
    pub id: String,
    pub name: String,
    pub version: String,
    pub developer: String,
    pub developer_url: Option<String>,
    pub short_description: String,
    pub long_description: String,
    pub accent_color: String,
    /// The outline icon (monochrome) and the color icon.
    pub small_icon: String,
    pub large_icon: String,
    pub categories: Vec<String>,
    pub screenshots: Vec<String>,
    pub video_url: Option<String>,
    pub privacy_url: Option<String>,
    pub terms_url: Option<String>,
    pub static_tabs: Vec<StaticTab>,
    pub valid_domains: Vec<String>,
    pub works_in: WorksIn,
    /// A personal tab or a personal bot: the app opens on its own from the app bar.
    pub has_personal_app: bool,
    pub has_tabs: bool,
    pub has_bot: bool,
    pub has_message_extension: bool,
    /// A Copilot agent (declarative or custom engine) rather than a classic app.
    pub is_agent: bool,
    /// Built into Teams (Activity, Chat, Calendar, …): never in the flyout or the Store.
    pub is_core: bool,
    pub is_microsoft: bool,
    /// Manifest permissions (`Identity`, `MessageTeamMembers`).
    pub permissions: Vec<String>,
    /// Resource-specific consent (RSC) permission names.
    pub rsc_permissions: Vec<String>,
    pub device_permissions: Vec<DevicePermission>,
    /// The definition as served. The install request posts it back (see [`crate::install`]).
    pub raw: Value,
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().filter(|s| !s.is_empty()).map(str::to_owned))
        .collect()
}

fn text(v: &Value, key: &str) -> String {
    str_of(v, key).unwrap_or_default()
}

/// Works-in from `metaCapabilities` (`{"staticTabs": {"scopes": ["Personal"]}, …}`). An app
/// works in meetings when it has a meeting extension or a tab for a meeting context.
fn works_in(v: &Value) -> WorksIn {
    let mut w = WorksIn::default();
    for (_, cap) in v["metaCapabilities"].as_object().into_iter().flatten() {
        for scope in strings(&cap["scopes"]) {
            match scope.to_ascii_lowercase().as_str() {
                "personal" => w.personal = true,
                "team" => w.team = true,
                "groupchat" => w.chat = true,
                _ => {}
            }
        }
    }
    w.meeting = v["metaCapabilities"]
        .get("meetingExtensionDefinition")
        .is_some()
        || v.get("meetingExtensionDefinition").is_some()
        || v["galleryTabs"].as_array().into_iter().flatten().any(|t| {
            strings(&t["context"])
                .iter()
                .any(|c| c.starts_with("Meeting"))
        });
    w
}

impl AppDefinition {
    /// `None` without an id.
    pub fn parse(v: &Value) -> Option<Self> {
        let id = str_of(v, "id")?;
        let caps = &v["metaCapabilities"];
        let has = |k: &str| caps.get(k).is_some() || v[k].as_array().is_some_and(|a| !a.is_empty());
        let static_tabs = v["staticTabs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                Some(StaticTab {
                    entity_id: str_of(t, "entityId")?,
                    name: text(t, "name"),
                    content_url: str_of(t, "contentUrl"),
                    website_url: str_of(t, "websiteUrl"),
                })
            })
            .collect();
        Some(AppDefinition {
            name: text(v, "name"),
            version: text(v, "version"),
            developer: text(v, "developerName"),
            developer_url: str_of(v, "developerUrl"),
            short_description: text(v, "shortDescription"),
            long_description: text(v, "longDescription"),
            accent_color: text(v, "accentColor"),
            small_icon: text(v, "smallImageUrl"),
            large_icon: text(v, "largeImageUrl"),
            categories: strings(&v["categories"]),
            screenshots: strings(&v["screenshotUrls"]),
            video_url: str_of(v, "videoUrl"),
            privacy_url: str_of(v, "privacyUrl"),
            terms_url: str_of(v, "termsOfUseUrl"),
            static_tabs,
            valid_domains: strings(&v["validDomains"]),
            works_in: works_in(v),
            has_personal_app: ["staticTabs", "bots"].iter().any(|k| {
                strings(&caps[*k]["scopes"])
                    .iter()
                    .any(|s| s.eq_ignore_ascii_case("personal"))
            }),
            has_tabs: has("staticTabs") || has("galleryTabs"),
            has_bot: has("bots"),
            has_message_extension: has("messageExtensions") || has("inputExtensions"),
            is_agent: has("copilotExtensions") || has("copilotPlugins"),
            is_core: v["isCoreApp"].as_bool().unwrap_or(false),
            is_microsoft: v["isMicrosoftOwned"].as_bool().unwrap_or(false),
            permissions: strings(&v["permissions"]),
            rsc_permissions: v["authorization"]["permissions"]["resourceSpecific"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| str_of(p, "name"))
                .collect(),
            device_permissions: strings(&v["devicePermissions"])
                .iter()
                .filter_map(|p| DevicePermission::parse(p))
                .collect(),
            raw: v.clone(),
            id,
        })
    }

    /// The personal tabs that open something (the manifest's `about` tab has no URL).
    pub fn personal_tabs(&self) -> impl Iterator<Item = &StaticTab> {
        self.static_tabs.iter().filter(|t| t.content_url.is_some())
    }

    /// Opens a tab of the app in Teams (the B class: the system browser).
    pub fn open_link(&self, entity_id: &str) -> String {
        format!(
            "https://teams.microsoft.com/l/entity/{}/{}",
            enc(&self.id),
            enc(entity_id)
        )
    }

    /// The app's Store page in Teams, for Share.
    pub fn share_link(&self) -> String {
        format!("https://teams.microsoft.com/l/app/{}", enc(&self.id))
    }
}

/// Percent-encodes a URL path segment.
pub(crate) fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// Every definition in an array (batched definitions answer), skipping entries without an id.
pub fn parse_definitions(v: &Value) -> Vec<AppDefinition> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(AppDefinition::parse)
        .collect()
}
