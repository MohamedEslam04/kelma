//! Per-host request header policy.
//!
//! Each Teams backend expects a different token and header set. [`headers_for`] is a pure
//! function so the policy can be unit-tested without network access.

use crate::client_info::{CLIENT_NAME, CLIENT_VERSION, TEAMS_ORIGIN, USER_AGENT};
use url::Url;

/// Access tokens currently held by a session. `None` means "not available".
#[derive(Clone, Default)]
pub struct Tokens {
    /// Skype token from the authz exchange.
    pub skype: Option<String>,
    /// Bearer token for `api.spaces.skype.com`.
    pub spaces: Option<String>,
    /// Bearer token for `chatsvcagg.teams.microsoft.com`.
    pub csa: Option<String>,
    /// Bearer token for `presence.teams.microsoft.com`.
    pub presence: Option<String>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print token values.
        f.debug_struct("Tokens")
            .field("skype", &self.skype.is_some())
            .field("spaces", &self.spaces.is_some())
            .field("csa", &self.csa.is_some())
            .field("presence", &self.presence.is_some())
            .finish()
    }
}

/// Which kind of backend a URL belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Chat service (`*.ng.msg.teams.microsoft.com` or `teams.microsoft.com/api/chatsvc`).
    ChatService,
    /// Chat service aggregator under `teams.microsoft.com/api/csa`.
    Csa,
    /// Middle tier and other `teams.microsoft.com` APIs.
    TeamsApi,
    /// Presence service.
    Presence,
    /// Trouter (real-time) endpoints.
    Trouter,
    /// AMS / async gateway (images and files).
    Ams,
    /// Anything else: no credentials are attached.
    Other,
}

/// Classifies a URL into a [`Backend`].
pub fn backend_of(url: &Url) -> Backend {
    let host = url.host_str().unwrap_or_default();
    let path = url.path();
    if host.ends_with("msg.teams.microsoft.com")
        || (host == "teams.microsoft.com" && path.starts_with("/api/chatsvc"))
    {
        Backend::ChatService
    } else if host == "teams.microsoft.com" && path.starts_with("/api/csa") {
        Backend::Csa
    } else if host == "teams.microsoft.com" {
        Backend::TeamsApi
    } else if host == "presence.teams.microsoft.com" {
        Backend::Presence
    } else if host.ends_with("trouter.teams.microsoft.com") || host.ends_with("trouter.skype.com") {
        Backend::Trouter
    } else if host.ends_with("asm.skype.com") || host.ends_with("asyncgw.teams.microsoft.com") {
        Backend::Ams
    } else {
        Backend::Other
    }
}

/// Headers to attach to a request for `url`.
///
/// Only hosts owned by Teams receive credentials. A URL on any other host gets the
/// user agent only, so a link inside a message can never leak a token.
pub fn headers_for(url: &Url, t: &Tokens, epid: &str) -> Vec<(&'static str, String)> {
    let mut h: Vec<(&'static str, String)> = vec![("User-Agent", USER_AGENT.to_string())];
    let backend = backend_of(url);
    if url.scheme() != "https" || backend == Backend::Other {
        return h;
    }
    h.push(("BehaviorOverride", "redirectAs404".into()));
    h.push(("Accept-Language", "en-US".into()));
    let skype = |h: &mut Vec<(&'static str, String)>| {
        if let Some(s) = &t.skype {
            h.push(("X-Skypetoken", s.clone()));
        }
    };
    let bearer = |h: &mut Vec<(&'static str, String)>, tok: &Option<String>| {
        if let Some(s) = tok {
            h.push(("Authorization", format!("Bearer {s}")));
        }
    };
    match backend {
        Backend::ChatService => {
            skype(&mut h);
            h.push(("X-Stratus-Caller", CLIENT_NAME.into()));
            h.push(("X-Stratus-Request", "abcd1234".into()));
            h.push(("Origin", TEAMS_ORIGIN.into()));
            h.push(("Referer", format!("{TEAMS_ORIGIN}/")));
            h.push(("Accept", "application/json; ver=1.0;".into()));
        }
        Backend::Csa => {
            bearer(&mut h, &t.csa);
            skype(&mut h);
        }
        Backend::TeamsApi => {
            bearer(&mut h, &t.spaces);
            skype(&mut h);
        }
        Backend::Presence => {
            if t.presence.is_some() {
                bearer(&mut h, &t.presence);
            } else {
                skype(&mut h);
            }
            h.push(("x-ms-client-user-agent", "Teams-V2-Desktop".into()));
            h.push(("x-ms-correlation-id", "1".into()));
            h.push(("x-ms-client-version", CLIENT_VERSION.into()));
            h.push(("x-ms-endpoint-id", epid.into()));
        }
        Backend::Trouter => skype(&mut h),
        Backend::Ams => {
            if let Some(s) = &t.skype {
                h.push(("Authorization", format!("skype_token {s}")));
            }
        }
        Backend::Other => {}
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks() -> Tokens {
        Tokens {
            skype: Some("SK".into()),
            spaces: Some("SP".into()),
            csa: Some("CS".into()),
            presence: None,
        }
    }
    fn get<'a>(h: &'a [(&'static str, String)], k: &str) -> Option<&'a str> {
        h.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn chat_service_gets_skypetoken() {
        let h = headers_for(
            &u("https://emea.ng.msg.teams.microsoft.com/v1/users/ME/conversations"),
            &toks(),
            "e",
        );
        assert_eq!(get(&h, "X-Skypetoken"), Some("SK"));
        assert_eq!(get(&h, "Origin"), Some("https://teams.microsoft.com"));
        assert_eq!(get(&h, "Authorization"), None);
    }

    #[test]
    fn csa_and_mt_get_their_bearers() {
        let h = headers_for(
            &u("https://teams.microsoft.com/api/csa/api/v1/teams/users/me"),
            &toks(),
            "e",
        );
        assert_eq!(get(&h, "Authorization"), Some("Bearer CS"));
        let h = headers_for(
            &u("https://teams.microsoft.com/api/mt/emea/beta/users/fetchShortProfile"),
            &toks(),
            "e",
        );
        assert_eq!(get(&h, "Authorization"), Some("Bearer SP"));
    }

    #[test]
    fn presence_falls_back_to_skypetoken() {
        let h = headers_for(
            &u("https://presence.teams.microsoft.com/v1/presence/getpresence/"),
            &toks(),
            "ep",
        );
        assert_eq!(get(&h, "X-Skypetoken"), Some("SK"));
        assert_eq!(get(&h, "x-ms-endpoint-id"), Some("ep"));
    }

    #[test]
    fn ams_uses_skype_token_scheme() {
        let h = headers_for(
            &u("https://as-api.asm.skype.com/v1/objects/0-x/views/imgo"),
            &toks(),
            "e",
        );
        assert_eq!(get(&h, "Authorization"), Some("skype_token SK"));
    }

    #[test]
    fn foreign_hosts_and_plain_http_get_no_credentials() {
        for url in [
            "https://example.com/a",
            "http://emea.ng.msg.teams.microsoft.com/v1",
            "https://evil.com/teams.microsoft.com",
        ] {
            let h = headers_for(&u(url), &toks(), "e");
            assert_eq!(h.len(), 1, "{url}");
            assert_eq!(h[0].0, "User-Agent");
        }
    }

    #[test]
    fn debug_never_prints_tokens() {
        assert!(!format!("{:?}", toks()).contains("SK"));
    }
}
