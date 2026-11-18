//! Authenticated client: owns tokens, refreshes them before they expire, and sends requests
//! with the right headers for each backend.

use crate::auth::{self, DeviceCode, SkypeToken};
use crate::client_info::{RESOURCE_CSA, RESOURCE_GRAPH, RESOURCE_PRESENCE, RESOURCE_SPACES};
use crate::error::{Error, Result};
use crate::http::{Tokens, headers_for};
use reqwest::Method;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;
use url::Url;

/// What an app persists to resume a session: store `refresh_token` in the system keyring and
/// `epid` anywhere (it is not secret).
#[derive(Clone)]
pub struct Session {
    pub tenant: String,
    pub refresh_token: String,
    /// Endpoint id, a UUID reused across runs so Teams does not hit its endpoint limit.
    pub epid: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("tenant", &self.tenant)
            .field("epid", &self.epid)
            .finish_non_exhaustive()
    }
}

/// Generates a new endpoint id.
pub fn new_epid() -> String {
    uuid::Uuid::new_v4().to_string()
}

struct State {
    tokens: Tokens,
    refresh_token: String,
    /// Earliest expiry across the tokens we hold.
    /// Wall-clock expiry. `Instant` would stop during system suspend and miss expiry after sleep.
    expires_at: SystemTime,
    chat_service: String,
    middle_tier: String,
    region: Option<String>,
}

/// Authenticated Teams client. Cheap to clone.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    tenant: String,
    epid: String,
    state: Arc<Mutex<State>>,
}

/// Installs the rustls crypto provider once (both reqwest and the websocket use rustls).
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

fn build_http() -> Result<reqwest::Client> {
    install_crypto_provider();
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()?)
}

/// Seconds before expiry at which tokens are refreshed.
const REFRESH_MARGIN: Duration = Duration::from_secs(300);

/// True when tokens expiring at `expires_at` must be refreshed at wall-clock time `now`.
fn refresh_due(expires_at: SystemTime, now: SystemTime) -> bool {
    expires_at <= now + REFRESH_MARGIN
}

impl Client {
    /// Starts a device-code sign-in. `on_code` is called once with the code to show to the user;
    /// the future resolves when the user has signed in.
    pub async fn sign_in_device_code(
        tenant: &str,
        epid: &str,
        on_code: impl FnOnce(&DeviceCode),
    ) -> Result<Client> {
        let http = build_http()?;
        let dc = auth::start_device_code(&http, tenant).await?;
        on_code(&dc);
        let tok = auth::poll_device_code(&http, tenant, &dc).await?;
        let refresh_token = tok
            .refresh_token
            .clone()
            .ok_or_else(|| Error::Auth("sign-in returned no refresh token".into()))?;
        Self::connect(
            http,
            Session {
                tenant: tenant.into(),
                refresh_token,
                epid: epid.into(),
            },
        )
        .await
    }

    /// Resumes a session from a stored refresh token.
    pub async fn resume(session: Session) -> Result<Client> {
        Self::connect(build_http()?, session).await
    }

    async fn connect(http: reqwest::Client, session: Session) -> Result<Client> {
        let state = State {
            tokens: Tokens::default(),
            refresh_token: session.refresh_token,
            expires_at: SystemTime::UNIX_EPOCH,
            chat_service: String::new(),
            middle_tier: String::new(),
            region: None,
        };
        let client = Client {
            http,
            tenant: session.tenant,
            epid: session.epid,
            state: Arc::new(Mutex::new(state)),
        };
        client.refresh_all().await?;
        Ok(client)
    }

    /// A client with no credentials, for demo mode and tests. Every network call fails.
    pub fn offline(epid: &str) -> Result<Client> {
        let state = State {
            tokens: Tokens::default(),
            refresh_token: String::new(),
            expires_at: SystemTime::now() + Duration::from_secs(365 * 24 * 3600),
            chat_service: "https://offline.invalid".into(),
            middle_tier: "https://offline.invalid".into(),
            region: None,
        };
        Ok(Client {
            http: build_http()?,
            tenant: String::new(),
            epid: epid.into(),
            state: Arc::new(Mutex::new(state)),
        })
    }

    /// Refreshes every access token and the skypetoken.
    pub async fn refresh_all(&self) -> Result<()> {
        let rt = self.state.lock().await.refresh_token.clone();
        let spaces = auth::refresh(&self.http, &self.tenant, &rt, RESOURCE_SPACES).await?;
        let rt = spaces.refresh_token.clone().unwrap_or(rt);
        let csa = auth::refresh(&self.http, &self.tenant, &rt, RESOURCE_CSA).await?;
        // Presence is optional: some tenants do not grant it. Fall back to the skypetoken.
        let presence = auth::refresh(&self.http, &self.tenant, &rt, RESOURCE_PRESENCE)
            .await
            .ok();
        // Graph is optional too: only file upload needs it.
        let graph = auth::refresh(&self.http, &self.tenant, &rt, RESOURCE_GRAPH)
            .await
            .ok();
        let sky: SkypeToken = auth::skypetoken(&self.http, &spaces.access_token).await?;
        let min_s = [
            Some(spaces.expires_in_s),
            Some(csa.expires_in_s),
            presence.as_ref().map(|p| p.expires_in_s),
            graph.as_ref().map(|g| g.expires_in_s),
            Some(sky.expires_in_s),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(3600);
        let mut st = self.state.lock().await;
        st.refresh_token = csa.refresh_token.clone().unwrap_or(rt);
        st.tokens = Tokens {
            skype: Some(sky.token),
            spaces: Some(spaces.access_token),
            csa: Some(csa.access_token),
            presence: presence.map(|p| p.access_token),
            graph: graph.map(|g| g.access_token),
        };
        st.expires_at = SystemTime::now() + Duration::from_secs(min_s);
        st.chat_service = sky.chat_service;
        st.middle_tier = sky.middle_tier;
        st.region = sky.region;
        tracing::info!(chat_service = %st.chat_service, region = ?st.region, ttl_s = min_s, "tokens refreshed");
        Ok(())
    }

    /// Refreshes tokens if any of them expires soon.
    pub async fn ensure_fresh(&self) -> Result<()> {
        let due = refresh_due(self.state.lock().await.expires_at, SystemTime::now());
        if due {
            self.refresh_all().await
        } else {
            Ok(())
        }
    }

    /// Current session data to persist (the refresh token rotates).
    pub async fn session(&self) -> Session {
        Session {
            tenant: self.tenant.clone(),
            refresh_token: self.state.lock().await.refresh_token.clone(),
            epid: self.epid.clone(),
        }
    }

    /// Endpoint id of this client.
    pub fn epid(&self) -> &str {
        &self.epid
    }

    /// Chat service base URL for this user's region.
    pub async fn chat_service(&self) -> String {
        self.state.lock().await.chat_service.clone()
    }

    /// Middle-tier base URL for this user's region.
    pub async fn middle_tier(&self) -> String {
        self.state.lock().await.middle_tier.clone()
    }

    /// A snapshot of the current tokens (for the trouter connection).
    pub async fn tokens(&self) -> Tokens {
        self.state.lock().await.tokens.clone()
    }

    /// The underlying HTTP client.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// Sends a request with credentials for its backend. Retries once after a 401 by refreshing.
    pub async fn request(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        extra: &[(&str, &str)],
    ) -> Result<(u16, String)> {
        self.ensure_fresh().await?;
        for attempt in 0..2 {
            let parsed =
                Url::parse(url).map_err(|e| Error::Protocol(format!("bad url {url}: {e}")))?;
            let tokens = self.tokens().await;
            let mut req = self.http.request(method.clone(), parsed.clone());
            for (k, v) in headers_for(&parsed, &tokens, &self.epid) {
                req = req.header(k, v);
            }
            for (k, v) in extra {
                req = req.header(*k, *v);
            }
            req = match body {
                Some(b) => req.json(b),
                None if method == Method::POST || method == Method::PUT => {
                    req.header("Content-Length", "0")
                }
                None => req,
            };
            let resp = req.send().await?;
            let status = resp.status().as_u16();
            if status == 401 && attempt == 0 {
                tracing::warn!(%url, "401, refreshing tokens");
                self.refresh_all().await?;
                continue;
            }
            return Ok((status, resp.text().await?));
        }
        unreachable!("loop returns on second attempt")
    }

    /// Sends a request and parses a JSON response; non-2xx statuses become errors.
    pub async fn json(
        &self,
        context: &'static str,
        method: Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let (status, text) = self.request(method, url, body, &[]).await?;
        if !(200..300).contains(&status) {
            return Err(Error::status(context, status, &text));
        }
        if text.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }

    /// Downloads bytes (images, avatars) with credentials for the URL's backend plus `extra` headers.
    pub async fn bytes(
        &self,
        url: &str,
        extra: &[(&str, String)],
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        self.ensure_fresh().await?;
        let parsed = Url::parse(url).map_err(|e| Error::Protocol(format!("bad url: {e}")))?;
        let tokens = self.tokens().await;
        let mut req = self.http.get(parsed.clone());
        for (k, v) in headers_for(&parsed, &tokens, &self.epid) {
            req = req.header(k, v);
        }
        for (k, v) in extra {
            req = req.header(*k, v);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(Error::status("download", status, ""));
        }
        let data = resp.bytes().await?;
        if data.len() > max_bytes {
            return Err(Error::Protocol(format!(
                "download larger than {max_bytes} bytes"
            )));
        }
        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_due_uses_wall_clock_margin() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let exp = t0 + Duration::from_secs(3600);
        assert!(!refresh_due(exp, t0));
        assert!(!refresh_due(exp, t0 + Duration::from_secs(3600 - 301)));
        assert!(refresh_due(exp, t0 + Duration::from_secs(3600 - 300)));
        // Eight hours of suspend: wall clock moved on, so the tokens are due.
        assert!(refresh_due(exp, t0 + Duration::from_secs(8 * 3600)));
    }
}
