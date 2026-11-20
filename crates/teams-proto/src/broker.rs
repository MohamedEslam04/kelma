//! Auth broker: one place that mints and caches an access token per audience.
//!
//! Every Microsoft 365 service kelma talks to accepts a bearer token minted from the same refresh
//! token for its own resource. [`Audience`] names those resources and [`TokenCache`] keeps one
//! token per audience with a wall-clock expiry, so a token is minted lazily on first use and
//! again only shortly before it expires. The refresh token that comes back from each mint is
//! rotated into the session by [`crate::client::Client::token_for`].

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// A resource (audience) that tokens can be minted for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Audience {
    /// `api.spaces.skype.com`: skypetoken exchange and the Teams middle tier.
    Spaces,
    /// `chatsvcagg.teams.microsoft.com`: teams and channels list.
    Csa,
    /// `presence.teams.microsoft.com`.
    Presence,
    /// Microsoft Graph.
    Graph,
    /// `substrate.office.com`: search, people suggestions, feed.
    Substrate,
    /// `substrate.office.com/sydney`: Copilot.
    Sydney,
    /// `outlook.office.com`: Outlook REST, calendar extras, mailbox settings.
    Outlook,
    /// A SharePoint host, for example `contoso.sharepoint.com` or `contoso-my.sharepoint.com`.
    SharePoint(String),
    /// `api.yammer.com`: Communities / Viva Engage.
    Yammer,
    /// The Viva Engage web app's own token (`https://www.yammer.com`), sent to the Engage
    /// GraphQL host `engage.cloud.microsoft` (storyline settings and posts).
    Engage,
    /// `service.flow.microsoft.com`: Workflows.
    Flow,
    /// `tasks.office.com`: Planner.
    Tasks,
    /// `onenote.com`.
    OneNote,
    /// `ic3.teams.office.com`: calling backend.
    Ic3,
    /// `loki.delve.office.com`: profile cards.
    Loki,
    /// `teams.microsoft.com` itself (region and authorization APIs).
    Teams,
    /// `templates.teams.microsoft.com`: the provisioning service behind private channels and
    /// channel members. Its token goes to `teams.microsoft.com/fabric/…`, not to its own host.
    Templates,
    /// The meeting content service (MCPS, Meeting Recaps): the web client's meeting artifacts
    /// resource, an app id rather than a host, as the mautrix bridge asks for it
    /// (`meetingContentOAuthScope`). Its token goes to `/api/mcps/` on the Teams web hosts.
    /// UNVERIFIED for kelma's client id.
    MeetingContent,
    /// A Teams app's own resource for app single sign-on, usually `api://<domain>/<app id>` (the
    /// `webApplicationInfo.resource` of its manifest). Build it with [`Audience::app`]. Its
    /// tokens are handed to the app (see `webhost`), never sent by kelma, so it owns no host.
    App(String),
}

impl Audience {
    /// Every fixed audience (SharePoint is per host and not listed).
    pub const FIXED: [Audience; 16] = [
        Audience::Spaces,
        Audience::Csa,
        Audience::Presence,
        Audience::Graph,
        Audience::Substrate,
        Audience::Sydney,
        Audience::Outlook,
        Audience::Yammer,
        Audience::Engage,
        Audience::Flow,
        Audience::Tasks,
        Audience::OneNote,
        Audience::Ic3,
        Audience::Loki,
        Audience::Teams,
        Audience::Templates,
    ];

    /// Resource URL used in the token request scope.
    pub fn resource(&self) -> String {
        match self {
            Audience::Spaces => crate::client_info::RESOURCE_SPACES.into(),
            Audience::Csa => crate::client_info::RESOURCE_CSA.into(),
            Audience::Presence => crate::client_info::RESOURCE_PRESENCE.into(),
            Audience::Graph => crate::client_info::RESOURCE_GRAPH.into(),
            Audience::Substrate => "https://substrate.office.com".into(),
            Audience::Sydney => "https://substrate.office.com/sydney".into(),
            Audience::Outlook => "https://outlook.office.com".into(),
            Audience::SharePoint(host) => format!("https://{host}"),
            Audience::Yammer => "https://api.yammer.com".into(),
            Audience::Engage => "https://www.yammer.com".into(),
            Audience::Flow => "https://service.flow.microsoft.com".into(),
            Audience::Tasks => "https://tasks.office.com".into(),
            Audience::OneNote => "https://onenote.com".into(),
            Audience::Ic3 => "https://ic3.teams.office.com".into(),
            Audience::Loki => "https://loki.delve.office.com".into(),
            Audience::Teams => "https://teams.microsoft.com".into(),
            Audience::Templates => "https://templates.teams.microsoft.com".into(),
            Audience::MeetingContent => "6bc3b958-689b-49f5-9006-36d165f30e00".into(),
            Audience::App(resource) => resource.clone(),
        }
    }

    /// The audience for a SharePoint host, or `None` when `host` is not a SharePoint host.
    pub fn sharepoint(host: &str) -> Option<Audience> {
        let host = host.to_ascii_lowercase();
        let label = host.strip_suffix(".sharepoint.com")?;
        if label.is_empty() || label.contains('.') {
            return None;
        }
        Some(Audience::SharePoint(host))
    }

    /// The audience for an app resource, or `None` when `resource` is not a single plain
    /// `api://` or `https://` resource. It goes into the token request's scope as it is, so
    /// whitespace (another scope), query, fragment, escapes and a trailing `/` or `/.default`
    /// are refused. Which app may get which resource is decided by the caller.
    pub fn app(resource: &str) -> Option<Audience> {
        let rest = resource
            .strip_prefix("api://")
            .or_else(|| resource.strip_prefix("https://"))?;
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | ':');
        if rest.is_empty()
            || !rest.chars().all(plain)
            || rest.ends_with('/')
            || rest.ends_with("/.default")
            || rest.starts_with('/')
        {
            return None;
        }
        Some(Audience::App(resource.to_string()))
    }

    /// Cache key: the resource URL.
    fn key(&self) -> String {
        self.resource()
    }
}

/// Seconds before expiry at which a cached token counts as stale.
pub const MARGIN: Duration = Duration::from_secs(300);

/// One access token per audience with its wall-clock expiry.
#[derive(Default)]
pub struct TokenCache {
    entries: HashMap<String, (String, SystemTime)>,
}

impl TokenCache {
    /// A token for `aud` that is still valid at `now` plus [`MARGIN`].
    pub fn get(&self, aud: &Audience, now: SystemTime) -> Option<String> {
        self.entry(aud, now).map(|(t, _)| t)
    }

    /// [`TokenCache::get`] with the token's wall-clock expiry.
    pub fn entry(&self, aud: &Audience, now: SystemTime) -> Option<(String, SystemTime)> {
        self.entries
            .get(&aud.key())
            .filter(|(_, exp)| *exp > now + MARGIN)
            .cloned()
    }

    /// Stores a token for `aud` that expires `expires_in_s` seconds after `now`.
    pub fn put(&mut self, aud: &Audience, token: String, now: SystemTime, expires_in_s: u64) {
        self.entries
            .insert(aud.key(), (token, now + Duration::from_secs(expires_in_s)));
    }

    /// Drops the token for `aud` (after the service rejected it).
    pub fn invalidate(&mut self, aud: &Audience) {
        self.entries.remove(&aud.key());
    }

    /// Drops every token.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of cached tokens, valid or not.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// An access token for an app resource with its wall-clock expiry.
#[derive(Clone)]
pub struct AppToken {
    pub access_token: String,
    pub expires_at: SystemTime,
}

impl std::fmt::Debug for AppToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppToken")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for TokenCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print token values.
        f.debug_list().entries(self.entries.keys()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for a in Audience::FIXED {
            assert!(seen.insert(a.resource()), "{a:?}");
            assert!(a.resource().starts_with("https://"));
        }
    }

    #[test]
    fn sharepoint_hosts_only() {
        assert_eq!(
            Audience::sharepoint("Contoso-My.SharePoint.com"),
            Some(Audience::SharePoint("contoso-my.sharepoint.com".into()))
        );
        assert_eq!(
            Audience::sharepoint("contoso.sharepoint.com")
                .unwrap()
                .resource(),
            "https://contoso.sharepoint.com"
        );
        assert_eq!(Audience::sharepoint("sharepoint.com"), None);
        assert_eq!(Audience::sharepoint("evil.com"), None);
        assert_eq!(Audience::sharepoint("a.b.sharepoint.com"), None);
        assert_eq!(Audience::sharepoint("x.sharepoint.com.evil.com"), None);
    }

    #[test]
    fn cache_respects_wall_clock_margin() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut c = TokenCache::default();
        assert!(c.is_empty());
        c.put(&Audience::Graph, "G".into(), t0, 3600);
        assert_eq!(c.get(&Audience::Graph, t0).as_deref(), Some("G"));
        assert_eq!(c.get(&Audience::Outlook, t0), None);
        // Still fresh just before the margin, stale inside it.
        assert!(
            c.get(&Audience::Graph, t0 + Duration::from_secs(3600 - 301))
                .is_some()
        );
        assert!(
            c.get(&Audience::Graph, t0 + Duration::from_secs(3600 - 300))
                .is_none()
        );
        // After a long suspend the token is stale.
        assert!(
            c.get(&Audience::Graph, t0 + Duration::from_secs(8 * 3600))
                .is_none()
        );
        c.invalidate(&Audience::Graph);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn app_resources_are_used_as_given_in_the_scope() {
        for r in [
            "api://app.contoso.com/0d5e1a2b-3c4d-4e5f-8a9b-0c1d2e3f4a5b",
            "api://app.contoso.com/botid-0d5e1a2b-3c4d-4e5f-8a9b-0c1d2e3f4a5b",
            "api://0d5e1a2b-3c4d-4e5f-8a9b-0c1d2e3f4a5b",
            "api://localhost:53000/0d5e1a2b-3c4d-4e5f-8a9b-0c1d2e3f4a5b",
            "https://graph.microsoft.com",
        ] {
            let aud = Audience::app(r).unwrap_or_else(|| panic!("{r}"));
            assert_eq!(aud, Audience::App(r.into()));
            assert_eq!(aud.resource(), r);
            assert_eq!(
                crate::client_info::scope_for(&aud.resource()),
                format!("{r}/.default openid profile offline_access")
            );
        }
    }

    #[test]
    fn app_resources_cannot_smuggle_scopes_or_odd_urls() {
        for r in [
            "",
            "api://",
            "https://",
            "api://x.com/a b",
            "api://x.com/a https://graph.microsoft.com/.default",
            "api://x.com/a\nb",
            "api://x.com/a?b=1",
            "api://x.com/a#b",
            "api://x.com/a%20b",
            "api://x.com/a/.default",
            "api://x.com/a/",
            "http://x.com",
            "javascript:alert(1)",
            "x.com/a",
        ] {
            assert_eq!(Audience::app(r), None, "{r:?}");
        }
    }

    #[test]
    fn app_tokens_are_cached_per_resource_with_their_expiry() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let a = Audience::app("api://a.contoso.com/1").unwrap();
        let b = Audience::app("api://b.contoso.com/1").unwrap();
        let mut c = TokenCache::default();
        c.put(&a, "A".into(), t0, 3600);
        assert_eq!(c.get(&a, t0).as_deref(), Some("A"));
        assert_eq!(c.get(&b, t0), None);
        assert_eq!(
            c.entry(&a, t0),
            Some(("A".to_string(), t0 + Duration::from_secs(3600)))
        );
        assert_eq!(c.entry(&a, t0 + Duration::from_secs(3600 - 300)), None);
        c.put(&b, "B".into(), t0, 60);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn app_token_debug_hides_the_token() {
        let t = AppToken {
            access_token: "SECRET".into(),
            expires_at: SystemTime::UNIX_EPOCH,
        };
        assert!(!format!("{t:?}").contains("SECRET"));
    }

    #[test]
    fn debug_hides_tokens() {
        let mut c = TokenCache::default();
        c.put(&Audience::Yammer, "SECRET".into(), SystemTime::now(), 60);
        assert!(!format!("{c:?}").contains("SECRET"));
    }
}
