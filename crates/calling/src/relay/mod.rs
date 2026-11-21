//! TURN credentials from `trap/tokens`, and the relay they are good for.
//!
//! `GET https://teams.microsoft.com/trap/tokens` with `api-version: 2`, `X-MS-Migration: True` and
//! the IC3 bearer answers `{tokens:[{realm, username, password}], expires}` (rusty-teams
//! `crates/calling/src/relay.rs`, MIT; teams-lite `NATIVE-CALLING.md` §2.6). The TURN hosts are
//! not in the answer: the web client reads them from a `relayConfig` (`Relay.Turn.fqdns`, ports)
//! that IronTeams has not found yet, and rusty-teams uses `gateway-<geo>.az.relay.teams.cloud.microsoft`
//! on udp 3478 and tcp/tls 443. Those gateways answer Allocate with `300 Try Alternate`, which
//! webrtc-rs does not follow, so [`resolve_ice_servers`] follows it first ([`allocate`]) and names
//! the relay that allocates by its address. Where UDP is blocked, [`resolve_tls_ice_servers`] finds
//! the relay over TLS on port 443 and points webrtc-rs at a loopback shim to it ([`tls`]).

pub mod allocate;
pub mod tls;

use crate::error::{Error, Result};
use crate::types::IceServer;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use teams_proto::Client;
use teams_proto::broker::Audience;
use tokio_rustls::rustls;

pub const TOKENS_URL: &str = "https://teams.microsoft.com/trap/tokens";
/// The relay rusty-teams defaults to (EU geo). Other geos are expected at
/// `gateway-<geo>.az.relay.teams.cloud.microsoft`.
pub const DEFAULT_RELAY_HOST: &str = "gateway-eu.az.relay.teams.cloud.microsoft";
/// The relay gateways' TURN port over UDP.
pub const RELAY_UDP_PORT: u16 = 3478;

/// One set of TURN long-term credentials.
#[derive(Clone, Deserialize)]
pub struct RelayCredentials {
    #[serde(default)]
    pub realm: String,
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for RelayCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the credentials.
        f.debug_struct("RelayCredentials")
            .field("realm", &self.realm)
            .finish_non_exhaustive()
    }
}

/// The `trap/tokens` answer.
#[derive(Debug, Clone)]
pub struct RelayGrant {
    pub credentials: Vec<RelayCredentials>,
    /// How long the credentials stay valid (`expires`, seconds; zero when not given).
    pub expires_in: Duration,
}

#[derive(Deserialize)]
struct TokensAnswer {
    tokens: Vec<RelayCredentials>,
    #[serde(default)]
    expires: u64,
}

/// Parses a `trap/tokens` answer. Fails when it holds no credentials.
pub fn parse_tokens(body: &str) -> Result<RelayGrant> {
    let a: TokensAnswer = serde_json::from_str(body)?;
    if a.tokens.is_empty() {
        return Err(Error::Protocol(
            "trap/tokens returned no relay token".into(),
        ));
    }
    Ok(RelayGrant {
        credentials: a.tokens,
        expires_in: Duration::from_secs(a.expires),
    })
}

/// One ICE server per credential set, each listing UDP, TCP and TLS on `relay_host`.
pub fn ice_servers(grant: &RelayGrant, relay_host: &str) -> Vec<IceServer> {
    grant
        .credentials
        .iter()
        .map(|c| IceServer {
            urls: vec![
                format!("turn:{relay_host}:3478?transport=udp"),
                format!("turn:{relay_host}:443?transport=tcp"),
                format!("turns:{relay_host}:443"),
            ],
            username: Some(c.username.clone()),
            credential: Some(c.password.clone()),
        })
        .collect()
}

/// Request headers: one credential (the IC3 bearer), `api-version: 2`, `X-MS-Migration`.
pub fn relay_headers(ic3_token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {ic3_token}")),
        ("api-version", "2".into()),
        ("X-MS-Migration", "True".into()),
        ("Accept", "application/json".into()),
        ("User-Agent", teams_proto::client_info::USER_AGENT.into()),
    ]
}

/// Fetches TURN credentials. The IC3 token is sent by hand: `Client::request_as` would refuse
/// `teams.microsoft.com` for the IC3 audience.
pub async fn fetch_grant(client: &Client) -> Result<RelayGrant> {
    let ic3 = client.token_for(&Audience::Ic3).await?;
    let mut req = client.http().get(TOKENS_URL);
    for (k, v) in relay_headers(&ic3) {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(teams_proto::Error::from)?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(teams_proto::Error::from)?;
    if !(200..300).contains(&status) {
        return Err(Error::Status {
            op: "trap_tokens",
            status,
            body: body.chars().take(400).collect(),
        });
    }
    parse_tokens(&body)
}

/// Fetches TURN credentials and turns them into ICE servers on `relay_host`.
pub async fn fetch_ice_servers(
    client: &Client,
    relay_host: &str,
) -> Result<(Vec<IceServer>, Duration)> {
    let grant = fetch_grant(client).await?;
    Ok((ice_servers(&grant, relay_host), grant.expires_in))
}

fn gateway(geo: &str) -> String {
    format!("gateway-{geo}.az.relay.teams.cloud.microsoft")
}

/// The relay gateways to try for an authz `region`, best first: the region's geo when one is
/// known, then the `world` gateway, then [`DEFAULT_RELAY_HOST`]. The geos are the gateway names
/// that resolve in DNS (`eu`, `us`, `in`, `world`); the region-to-geo map is a guess, which is
/// why every list ends in the gateways that serve anyone.
pub fn relay_hosts(region: &str) -> Vec<String> {
    let geo = match region.to_ascii_lowercase().as_str() {
        "amer" | "noam" | "us" | "ca" | "latam" | "br" => Some("us"),
        "emea" | "eu" | "europe" | "uk" | "fr" | "de" | "ch" | "no" | "se" => Some("eu"),
        "in" => Some("in"),
        _ => None,
    };
    let mut hosts: Vec<String> = geo.into_iter().map(gateway).collect();
    for fallback in [gateway("world"), DEFAULT_RELAY_HOST.to_owned()] {
        if !hosts.contains(&fallback) {
            hosts.push(fallback);
        }
    }
    hosts
}

/// One ICE server per credential set naming the relay at `relay` (an address [`allocate::find_relay`]
/// returned) over UDP, the only TURN transport webrtc-rs 0.21.1 speaks.
pub fn resolved_ice_servers(grant: &RelayGrant, relay: SocketAddr) -> Vec<IceServer> {
    grant
        .credentials
        .iter()
        .map(|c| IceServer {
            urls: vec![format!("turn:{relay}?transport=udp")],
            username: Some(c.username.clone()),
            credential: Some(c.password.clone()),
        })
        .collect()
}

/// The first address of each family `host` resolves to on `port`, IPv4 first.
async fn host_addrs(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    let mut addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| Error::Protocol(format!("resolve {host}: {e}")))?
        .collect();
    addrs.sort_by_key(|a| a.is_ipv6());
    addrs.dedup_by_key(|a| a.is_ipv6());
    Ok(addrs)
}

/// The relay `host` leads to: its first address of each family is probed (IPv4 first).
async fn probe_host(host: &str, credentials: &RelayCredentials) -> Result<SocketAddr> {
    let mut last = Error::Protocol(format!("{host} has no address"));
    for addr in host_addrs(host, RELAY_UDP_PORT).await? {
        match allocate::find_relay(addr, credentials).await {
            Ok(relay) => return Ok(relay),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// [`probe_host`] over TLS to port 443, the certificate checked for `host`.
async fn probe_host_tls(
    host: &str,
    credentials: &RelayCredentials,
    config: &Arc<rustls::ClientConfig>,
) -> Result<tls::TlsRelay> {
    let mut last = Error::Protocol(format!("{host} has no address"));
    for addr in host_addrs(host, tls::RELAY_TLS_PORT).await? {
        match tls::find_relay_tls(addr, host, credentials, Arc::clone(config)).await {
            Ok(relay) => return Ok(relay),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Fetches TURN credentials and finds the relay that allocates with them, starting from
/// `relay_host` when given, else from [`relay_hosts`] for the authz `region`. When no gateway
/// leads to a relay, the ICE servers name the first gateway as before (webrtc-rs will log the
/// `300 Try Alternate`), so a call still connects wherever host candidates do.
pub async fn resolve_ice_servers(
    client: &Client,
    region: &str,
    relay_host: Option<&str>,
) -> Result<(Vec<IceServer>, Duration)> {
    let grant = fetch_grant(client).await?;
    let hosts = match relay_host {
        Some(h) => vec![h.to_owned()],
        None => relay_hosts(region),
    };
    let credentials = grant
        .credentials
        .first()
        .ok_or_else(|| Error::Protocol("relay grant holds no credentials".into()))?;
    for host in &hosts {
        match probe_host(host, credentials).await {
            Ok(relay) => {
                tracing::info!(%host, %relay, "TURN relay found");
                return Ok((resolved_ice_servers(&grant, relay), grant.expires_in));
            }
            Err(e) => tracing::warn!(%host, "no TURN relay: {e}"),
        }
    }
    Ok((ice_servers(&grant, &hosts[0]), grant.expires_in))
}

/// [`resolve_ice_servers`] for a network that blocks UDP: finds the relay over TLS on port 443
/// and starts a [`tls::TlsShim`] to it, which the ICE servers name. The shim must be kept for as
/// long as any peer connection built from them. Fails when no gateway takes a TLS allocation.
pub async fn resolve_tls_ice_servers(
    client: &Client,
    region: &str,
    relay_host: Option<&str>,
) -> Result<(Vec<IceServer>, Duration, tls::TlsShim)> {
    let grant = fetch_grant(client).await?;
    let hosts = match relay_host {
        Some(h) => vec![h.to_owned()],
        None => relay_hosts(region),
    };
    let credentials = grant
        .credentials
        .first()
        .ok_or_else(|| Error::Protocol("relay grant holds no credentials".into()))?;
    let config = tls::client_config();
    let mut last = Error::Protocol("no TURN relay over TLS".into());
    for host in &hosts {
        match probe_host_tls(host, credentials, &config).await {
            Ok(relay) => {
                tracing::info!(%host, relay = %relay.addr, "TURN relay over TLS found");
                let shim = tls::TlsShim::start(relay, config).await?;
                let ice = resolved_ice_servers(&grant, shim.local_addr());
                return Ok((ice, grant.expires_in, shim));
            }
            Err(e) => {
                tracing::warn!(%host, "no TURN relay over TLS: {e}");
                last = e;
            }
        }
    }
    Err(last)
}
