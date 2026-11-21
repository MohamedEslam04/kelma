//! Where the NGC conversation service lives, how requests to it are routed, and its headers.

use crate::error::{Error, Result};
use serde_json::Value;

/// `x-microsoft-skype-client`: the calling format (`SkypeSpaces/<build>/<platform>/TsCallingVersion=...`),
/// not the Skype-era one the messaging services want. mautrix-teams and ost send this shape.
pub const CLIENT_HEADER: &str = "SkypeSpaces/1415/ironteams/TsCallingVersion=2025.49.01.15";

/// Requests to call-controller and conversation backends (`*.cc.skype.com`, `*.conv.skype.com`)
/// go through this proxy as `<FLIGHTPROXY_EP>/<host>/<path>`.
pub const FLIGHTPROXY_EP: &str = "https://api.flightproxy.teams.microsoft.com/api/v2/ep";

/// The authz endpoint that names the calling services for this user.
pub const AUTHZ_URL: &str = "https://teams.microsoft.com/api/authsvc/v1.0/authz";

/// The calling plane for this user, from the authz response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallingEndpoints {
    /// `regionGtms.calling_conversationServiceUrl` (`https://api-<geo>.flightproxy.teams.microsoft.com/api/v2/epconv`).
    pub conversation_service_url: String,
    /// `ms-teams-region`.
    pub region: String,
    /// `ms-teams-partition`.
    pub partition: String,
    /// `ms-teams-ring`.
    pub ring: String,
}

fn text(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Parses the `authsvc/v1.0/authz` response (teams-proto does not keep the calling keys).
pub fn parse_authz(body: &str) -> Result<CallingEndpoints> {
    let v: Value = serde_json::from_str(body)?;
    let conversation_service_url = text(&v, "/regionGtms/calling_conversationServiceUrl")
        .ok_or_else(|| {
            Error::Protocol("authz has no regionGtms.calling_conversationServiceUrl".into())
        })?;
    Ok(CallingEndpoints {
        conversation_service_url,
        region: text(&v, "/region").unwrap_or_default(),
        partition: text(&v, "/partition").unwrap_or_default(),
        ring: "general".into(),
    })
}

/// Host and the rest (`/path?query`) of an https URL, without a port.
fn split_https(url: &str) -> Result<(&str, &str)> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| Error::Protocol(format!("refusing non-https link {url}")))?;
    let cut = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(cut);
    let host = authority.split(':').next().unwrap_or(authority);
    Ok((host, tail))
}

fn is_hostname(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

// Adapted from rusty-teams (MIT), crates/calling/src/signaling.rs `routed_url` / `ensure_teams_url`.
/// The URL to actually send a request for `url` to: `*.skype.com` backends are wrapped in the
/// flightproxy, Teams hosts pass through, anything else is refused (the IC3 bearer must never
/// leave Microsoft's calling hosts; links come from server answers and pushes).
pub fn routed_url(url: &str) -> Result<String> {
    let (host, tail) = split_https(url)?;
    let host = host.to_ascii_lowercase();
    if !is_hostname(&host) {
        return Err(Error::Protocol(format!("refusing link to host {host}")));
    }
    // Already proxied: the consumer flightproxy (`api.flightproxy.skype.com/api/v2/cp/...`, where a
    // personal account's meeting lives) is kept as the service handed it, never wrapped again.
    if host.ends_with(".flightproxy.skype.com") {
        return Ok(url.to_owned());
    }
    if host.ends_with(".skype.com") {
        return Ok(format!("{FLIGHTPROXY_EP}/{host}{tail}"));
    }
    if host == "teams.microsoft.com" || host.ends_with(".teams.microsoft.com") {
        return Ok(url.to_owned());
    }
    Err(Error::Protocol(format!("refusing link to host {host}")))
}

/// Headers for a call-control request: exactly one credential (the IC3 bearer, never the
/// skypetoken too: two credentials are refused with an empty 400), the routing trio, and no
/// `api-version`.
pub fn call_headers(
    ep: &CallingEndpoints,
    chain_id: &str,
    message_id: &str,
    ic3_token: &str,
) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {ic3_token}")),
        ("Content-Type", "application/json".into()),
        ("User-Agent", teams_proto::client_info::USER_AGENT.into()),
        ("Referer", "https://teams.microsoft.com/".into()),
        ("x-microsoft-skype-chain-id", chain_id.into()),
        ("x-microsoft-skype-message-id", message_id.into()),
        ("x-microsoft-skype-client", CLIENT_HEADER.into()),
        ("ms-teams-partition", ep.partition.clone()),
        ("ms-teams-region", ep.region.clone()),
        ("ms-teams-ring", ep.ring.clone()),
        ("x-ms-migration", "True".into()),
    ]
}
