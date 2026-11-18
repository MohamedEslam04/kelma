//! TURN credentials from `trap/tokens`.
//!
//! `GET https://teams.microsoft.com/trap/tokens` with `api-version: 2`, `X-MS-Migration: True` and
//! the IC3 bearer answers `{tokens:[{realm, username, password}], expires}` (rusty-teams
//! `crates/calling/src/relay.rs`, MIT; teams-lite `NATIVE-CALLING.md` §2.6). The TURN hosts are
//! not in the answer: rusty-teams uses `gateway-<geo>.az.relay.teams.cloud.microsoft` on udp 3478
//! and tcp/tls 443.

use crate::error::{Error, Result};
use crate::types::IceServer;
use serde::Deserialize;
use std::time::Duration;
use teams_proto::Client;
use teams_proto::broker::Audience;

pub const TOKENS_URL: &str = "https://teams.microsoft.com/trap/tokens";
/// The relay rusty-teams defaults to (EU geo). Other geos are expected at
/// `gateway-<geo>.az.relay.teams.cloud.microsoft`.
pub const DEFAULT_RELAY_HOST: &str = "gateway-eu.az.relay.teams.cloud.microsoft";

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
