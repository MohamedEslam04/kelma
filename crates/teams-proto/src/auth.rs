//! Sign-in: OAuth device code, refresh tokens and the skypetoken exchange.

use crate::client_info::{
    AUTHORITY, CLIENT_ID, FALLBACK_CHAT_SERVICE, FALLBACK_MIDDLE_TIER, FALLBACK_PRESENCE,
    RESOURCE_SPACES, USER_AGENT, scope_for,
};
use crate::error::{Error, Result};
use serde_json::Value;
use std::time::Duration;

/// A pending device-code sign-in. Show `user_code` and `verification_url` to the user.
#[derive(Debug, Clone)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval_s: u64,
    pub expires_in_s: u64,
    pub message: String,
}

/// An OAuth token response.
#[derive(Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_s: u64,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("expires_in_s", &self.expires_in_s)
            .finish_non_exhaustive()
    }
}

/// Result of one device-code poll.
#[derive(Debug)]
pub enum PollOutcome {
    /// The user has not finished signing in yet.
    Pending,
    /// Poll less often.
    SlowDown,
    /// Signed in.
    Done(TokenResponse),
}

/// Result of the skypetoken exchange, including where this user's chat data lives.
#[derive(Clone)]
pub struct SkypeToken {
    pub token: String,
    pub expires_in_s: u64,
    /// Chat service base URL, e.g. `https://emea.ng.msg.teams.microsoft.com`.
    pub chat_service: String,
    /// Middle-tier base URL, e.g. `https://teams.microsoft.com/api/mt/emea`.
    pub middle_tier: String,
    /// Presence service base URL with its version, e.g.
    /// `https://teams.cloud.microsoft/ups/noam/v1` (`regionGtms.presenceUPS`).
    pub presence: String,
    pub region: Option<String>,
}

impl std::fmt::Debug for SkypeToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkypeToken")
            .field("expires_in_s", &self.expires_in_s)
            .field("chat_service", &self.chat_service)
            .field("middle_tier", &self.middle_tier)
            .field("presence", &self.presence)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

/// Numbers in Microsoft responses are sometimes sent as strings.
fn num(v: Option<&Value>) -> Option<u64> {
    match v? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_owned)
}

/// Parses the device-code start response.
pub fn parse_device_code(body: &str) -> Result<DeviceCode> {
    let v: Value = serde_json::from_str(body)?;
    if let Some(err) = s(v.get("error")) {
        return Err(Error::Auth(format!(
            "{err}: {}",
            s(v.get("error_description")).unwrap_or_default()
        )));
    }
    Ok(DeviceCode {
        device_code: s(v.get("device_code")).ok_or_else(|| Error::Auth("no device_code".into()))?,
        user_code: s(v.get("user_code")).ok_or_else(|| Error::Auth("no user_code".into()))?,
        verification_url: s(v.get("verification_url"))
            .or_else(|| s(v.get("verification_uri")))
            .unwrap_or_else(|| "https://microsoft.com/devicelogin".into()),
        interval_s: num(v.get("interval")).unwrap_or(5).max(1),
        expires_in_s: num(v.get("expires_in")).unwrap_or(900),
        message: s(v.get("message")).unwrap_or_default(),
    })
}

/// Parses a token endpoint response (device-code poll or refresh).
pub fn parse_token_response(body: &str) -> Result<PollOutcome> {
    let v: Value = serde_json::from_str(body)?;
    if let Some(err) = s(v.get("error")) {
        return match err.as_str() {
            "authorization_pending" => Ok(PollOutcome::Pending),
            "slow_down" => Ok(PollOutcome::SlowDown),
            "expired_token" | "code_expired" => Err(Error::DeviceCodeExpired),
            _ => {
                let desc = s(v.get("error_description")).unwrap_or_default();
                let first_line = desc.lines().next().unwrap_or_default().to_string();
                Err(Error::Auth(format!("{err}: {first_line}")))
            }
        };
    }
    Ok(PollOutcome::Done(TokenResponse {
        access_token: s(v.get("access_token"))
            .ok_or_else(|| Error::Auth("no access_token".into()))?,
        refresh_token: s(v.get("refresh_token")),
        expires_in_s: num(v.get("expires_in")).unwrap_or(3600),
    }))
}

/// Parses the `authsvc/v1.0/authz` response.
pub fn parse_authz(body: &str) -> Result<SkypeToken> {
    let v: Value = serde_json::from_str(body)?;
    let tokens = v.get("tokens");
    let token = tokens
        .and_then(|t| t.get("skypeToken"))
        .or_else(|| v.get("skypeToken").and_then(|t| t.get("skypetoken")))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            let msg = s(v.get("message")).or_else(|| {
                v.pointer("/status/text")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
            Error::Auth(format!(
                "authz returned no skype token: {}",
                msg.unwrap_or_default()
            ))
        })?
        .to_string();
    let expires_in_s = num(tokens.and_then(|t| t.get("expiresIn"))).unwrap_or(3600);
    let region = s(v.get("region"));
    let gtms = v.get("regionGtms");
    let trim = |u: String| u.trim_end_matches('/').to_string();
    let chat_service = s(gtms.and_then(|g| g.get("chatService")))
        .map(trim)
        .or_else(|| {
            region
                .as_ref()
                .map(|r| format!("https://{r}.ng.msg.teams.microsoft.com"))
        })
        .unwrap_or_else(|| FALLBACK_CHAT_SERVICE.into());
    let middle_tier = s(gtms.and_then(|g| g.get("middleTier")))
        .map(trim)
        .or_else(|| {
            region
                .as_ref()
                .map(|r| format!("https://teams.microsoft.com/api/mt/{r}"))
        })
        .unwrap_or_else(|| FALLBACK_MIDDLE_TIER.into());
    // Teams web sends presence to the regional unified presence service; the global host
    // refuses this client's presence token.
    let presence = s(gtms.and_then(|g| g.get("presenceUPS")))
        .map(|u| format!("{}/v1", trim(u)))
        .unwrap_or_else(|| FALLBACK_PRESENCE.into());
    Ok(SkypeToken {
        token,
        expires_in_s,
        chat_service,
        middle_tier,
        presence,
        region,
    })
}

fn token_url(tenant: &str, v2: bool) -> String {
    if v2 {
        format!("{AUTHORITY}/{tenant}/oauth2/v2.0/token")
    } else {
        format!("{AUTHORITY}/{tenant}/oauth2/token")
    }
}

/// Starts a device-code sign-in.
pub async fn start_device_code(http: &reqwest::Client, tenant: &str) -> Result<DeviceCode> {
    let resp = http
        .post(format!("{AUTHORITY}/{tenant}/oauth2/devicecode"))
        .header("User-Agent", USER_AGENT)
        .form(&[("client_id", CLIENT_ID), ("resource", RESOURCE_SPACES)])
        .send()
        .await?;
    let body = resp.text().await?;
    parse_device_code(&body)
}

/// Polls until the user finishes signing in, the code expires, or sign-in fails.
pub async fn poll_device_code(
    http: &reqwest::Client,
    tenant: &str,
    dc: &DeviceCode,
) -> Result<TokenResponse> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in_s);
    let mut interval = dc.interval_s;
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if tokio::time::Instant::now() > deadline {
            return Err(Error::DeviceCodeExpired);
        }
        let resp = http
            .post(token_url(tenant, false))
            .header("User-Agent", USER_AGENT)
            .form(&[
                ("client_id", CLIENT_ID),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("code", dc.device_code.as_str()),
            ])
            .send()
            .await?;
        match parse_token_response(&resp.text().await?)? {
            PollOutcome::Pending => {}
            PollOutcome::SlowDown => interval += 5,
            PollOutcome::Done(t) => return Ok(t),
        }
    }
}

/// Exchanges a refresh token for an access token for `resource`.
pub async fn refresh(
    http: &reqwest::Client,
    tenant: &str,
    refresh_token: &str,
    resource: &str,
) -> Result<TokenResponse> {
    let scope = scope_for(resource);
    let resp = http
        .post(token_url(tenant, true))
        .header("User-Agent", USER_AGENT)
        .form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", scope.as_str()),
        ])
        .send()
        .await?;
    match parse_token_response(&resp.text().await?)? {
        PollOutcome::Done(t) => Ok(t),
        _ => Err(Error::Auth(
            "unexpected pending response to a refresh".into(),
        )),
    }
}

/// Exchanges the `api.spaces.skype.com` access token for a skypetoken.
pub async fn skypetoken(http: &reqwest::Client, spaces_access_token: &str) -> Result<SkypeToken> {
    let resp = http
        .post("https://teams.microsoft.com/api/authsvc/v1.0/authz")
        .header("User-Agent", USER_AGENT)
        .header("Authorization", format!("Bearer {spaces_access_token}"))
        .header("Accept", "application/json; ver=1.0")
        .header("Content-Length", "0")
        .send()
        .await?;
    let status = resp.status().as_u16();
    let body = resp.text().await?;
    if status >= 400 {
        return Err(Error::status("authz", status, &body));
    }
    parse_authz(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_code_accepts_string_numbers() {
        let dc = parse_device_code(
            r#"{"user_code":"GRUE55CDY","device_code":"DC","verification_url":"https://microsoft.com/devicelogin",
                "expires_in":"900","interval":"5","message":"To sign in..."}"#,
        )
        .unwrap();
        assert_eq!(dc.user_code, "GRUE55CDY");
        assert_eq!(dc.interval_s, 5);
        assert_eq!(dc.expires_in_s, 900);
    }

    #[test]
    fn device_code_v2_uri_field() {
        let dc = parse_device_code(
            r#"{"user_code":"A","device_code":"B","verification_uri":"https://login.microsoft.com/device","expires_in":600,"interval":1}"#,
        )
        .unwrap();
        assert_eq!(dc.verification_url, "https://login.microsoft.com/device");
    }

    #[test]
    fn token_poll_states() {
        assert!(matches!(
            parse_token_response(r#"{"error":"authorization_pending"}"#).unwrap(),
            PollOutcome::Pending
        ));
        assert!(matches!(
            parse_token_response(r#"{"error":"slow_down"}"#).unwrap(),
            PollOutcome::SlowDown
        ));
        assert!(matches!(
            parse_token_response(r#"{"error":"expired_token"}"#),
            Err(Error::DeviceCodeExpired)
        ));
        assert!(matches!(
            parse_token_response(r#"{"error":"invalid_grant","error_description":"AADSTS70000: bad\r\nTrace ID: x"}"#),
            Err(Error::Auth(m)) if m == "invalid_grant: AADSTS70000: bad"
        ));
        match parse_token_response(
            r#"{"access_token":"AT","refresh_token":"RT","expires_in":"3599"}"#,
        )
        .unwrap()
        {
            PollOutcome::Done(t) => {
                assert_eq!(t.access_token, "AT");
                assert_eq!(t.refresh_token.as_deref(), Some("RT"));
                assert_eq!(t.expires_in_s, 3599);
                assert!(!format!("{t:?}").contains("AT"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn authz_with_region_gtms() {
        let t = parse_authz(
            r#"{"tokens":{"skypeToken":"SKY","expiresIn":86399,"tokenType":"SkypeToken"},"region":"emea","partition":"emea02",
                "regionGtms":{"chatService":"https://emea.ng.msg.teams.microsoft.com/","middleTier":"https://teams.microsoft.com/api/mt/emea"}}"#,
        )
        .unwrap();
        assert_eq!(t.token, "SKY");
        assert_eq!(t.expires_in_s, 86399);
        assert_eq!(t.chat_service, "https://emea.ng.msg.teams.microsoft.com");
        assert_eq!(t.middle_tier, "https://teams.microsoft.com/api/mt/emea");
        assert!(!format!("{t:?}").contains("SKY"));
        assert_eq!(t.presence, FALLBACK_PRESENCE);
    }

    #[test]
    fn authz_reads_the_regional_presence_service() {
        let t = parse_authz(
            r#"{"tokens":{"skypeToken":"S","expiresIn":100},"region":"amer",
                "regionGtms":{"presenceUPS":"https://teams.cloud.microsoft/ups/noam/"}}"#,
        )
        .unwrap();
        assert_eq!(t.presence, "https://teams.cloud.microsoft/ups/noam/v1");
    }

    #[test]
    fn authz_falls_back_to_region_then_default() {
        let t = parse_authz(r#"{"tokens":{"skypeToken":"S","expiresIn":"100"},"region":"amer"}"#)
            .unwrap();
        assert_eq!(t.chat_service, "https://amer.ng.msg.teams.microsoft.com");
        let t = parse_authz(r#"{"skypeToken":{"skypetoken":"S"}}"#).unwrap();
        assert_eq!(t.chat_service, FALLBACK_CHAT_SERVICE);
        assert!(parse_authz(r#"{"status":{"code":403,"text":"denied"}}"#).is_err());
    }
}
