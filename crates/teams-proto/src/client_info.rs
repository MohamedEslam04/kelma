//! Constants that identify the client to Teams.
//!
//! Teams rotates these over time. Keep them in this one file so updating them is a one-line change.

/// Azure AD application id of the Teams web/desktop client (work and school accounts).
pub const CLIENT_ID: &str = "1fec8e78-bce4-4aaf-ab1b-5451cc387264";
/// Login authority.
pub const AUTHORITY: &str = "https://login.microsoftonline.com";
/// Default tenant segment for work and school accounts.
pub const DEFAULT_TENANT: &str = "organizations";

/// Resource behind the skypetoken exchange and most `teams.microsoft.com/api/mt` calls.
pub const RESOURCE_SPACES: &str = "https://api.spaces.skype.com";
/// Chat service aggregator (teams and channels list).
pub const RESOURCE_CSA: &str = "https://chatsvcagg.teams.microsoft.com";
/// Presence service.
pub const RESOURCE_PRESENCE: &str = "https://presence.teams.microsoft.com";
/// Microsoft Graph, for OneDrive file upload (`Files.ReadWrite.All` is granted to this client).
pub const RESOURCE_GRAPH: &str = "https://graph.microsoft.com";

/// Client version string sent in several headers and the trouter registration.
pub const CLIENT_VERSION: &str = "49/25113001312";
/// Client name used together with [`CLIENT_VERSION`].
pub const CLIENT_NAME: &str = "skypeteams";
/// Trouter `tc.cv` value.
pub const TROUTER_TCCV: &str = "2024.23.01.2";
/// Browser user agent the endpoints expect.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0 Teams/24165.1410.2974.6689/49";

/// Origin of the Teams web app, sent as `Origin`/`Referer`.
pub const TEAMS_ORIGIN: &str = "https://teams.microsoft.com";
/// Chat service used when the authz response has no region information.
pub const FALLBACK_CHAT_SERVICE: &str = "https://apac.ng.msg.teams.microsoft.com";
/// Middle tier used when the authz response has no region information.
pub const FALLBACK_MIDDLE_TIER: &str = "https://teams.microsoft.com/api/mt/apac";
/// Presence service used when the sign-in answer names no regional one.
pub const FALLBACK_PRESENCE: &str = "https://presence.teams.microsoft.com/v1";

/// Scope string for a v2 token request for `resource`.
pub fn scope_for(resource: &str) -> String {
    format!("{resource}/.default openid profile offline_access")
}
