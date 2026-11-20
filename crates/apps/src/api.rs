//! Calls to the Teams app service. The web client sends these to `regionGtms.appService` or,
//! behind a flag, to `regionGtms.middleTier`; both answer the same `beta/…/apps/…` paths
//! (verified live 2026-11-19), so kelma uses the middle tier it already knows. Both hosts are
//! on `teams.microsoft.com`, which gets the Spaces bearer and the skypetoken from the client.

use reqwest::Method;
use serde_json::Value;
use teams_proto::{Client, Error, Result};

use crate::install::Request;

/// The `x-ms-client-caller` header the app service expects on every request.
pub const CALLER: &str = "kelma";

/// The app service base URL for this user (the middle tier, without a trailing slash).
pub async fn base(client: &Client) -> String {
    client.middle_tier().await.trim_end_matches('/').to_owned()
}

/// Sends a change to the app service (install, uninstall, update, pin). The request's URL must
/// be on the app service host the client already uses; the answer is returned as JSON (`Null`
/// when empty), any other status than 2xx is an error.
pub async fn send(client: &Client, context: &'static str, req: &Request) -> Result<Value> {
    let (status, text) = client
        .request(
            req.method.clone(),
            &req.url,
            req.body.as_ref(),
            &[("x-ms-client-caller", CALLER)],
        )
        .await?;
    if !(200..300).contains(&status) {
        return Err(Error::status(context, status, &text));
    }
    if text.trim().is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_str(&text)?)
    }
}

/// Sends a read to the app service and parses the JSON answer. Some reads are POSTs (search,
/// the Store home, batched definitions, the aggregated entitlements); none of them changes
/// anything.
pub(crate) async fn read(
    client: &Client,
    context: &'static str,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let url = format!("{}/{path}", base(client).await);
    let (status, text) = client
        .request(method, &url, body, &[("x-ms-client-caller", CALLER)])
        .await?;
    if !(200..300).contains(&status) {
        return Err(Error::status(context, status, &text));
    }
    if text.trim().is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_str(&text)?)
    }
}
