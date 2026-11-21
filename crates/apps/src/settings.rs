//! Store settings (the gear): the tenant's app policy and the per-app device permissions.
//!
//! Teams keeps device permission grants on the device, not on the server, so IronTeams keeps them
//! in its own config ([`Grants`]); the web host asks them before an app uses the camera,
//! location, notifications, MIDI or external links.

use std::collections::BTreeMap;

use reqwest::Method;
use serde_json::{Map, Value};
use teams_proto::{Client, Result};

use crate::api;
use crate::model::DevicePermission;

/// What the admin allows (`beta/me/settings/appPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppPolicy {
    /// Custom app upload.
    pub sideloading_allowed: bool,
    /// Pinning and reordering the app bar.
    pub user_pinning_allowed: bool,
}

/// Parses the app policy. Missing fields fall back to the Teams defaults (pinning allowed,
/// upload not).
pub fn parse_app_policy(v: &Value) -> AppPolicy {
    let p = &v["value"];
    AppPolicy {
        sideloading_allowed: p["isSideloadingAllowed"].as_bool().unwrap_or(false),
        user_pinning_allowed: p["isUserPinningAllowed"].as_bool().unwrap_or(true),
    }
}

pub async fn app_policy(client: &Client) -> Result<AppPolicy> {
    let v = api::read(
        client,
        "app policy",
        Method::GET,
        "beta/me/settings/appPolicy",
        None,
    )
    .await?;
    Ok(parse_app_policy(&v))
}

/// The user's device permission grants, per app id. Anything not granted is denied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants(BTreeMap<String, BTreeMap<DevicePermission, bool>>);

impl Grants {
    pub fn allowed(&self, app_id: &str, p: DevicePermission) -> bool {
        self.0
            .get(app_id)
            .and_then(|m| m.get(&p))
            .copied()
            .unwrap_or(false)
    }

    pub fn set(&mut self, app_id: &str, p: DevicePermission, allowed: bool) {
        self.0
            .entry(app_id.to_owned())
            .or_default()
            .insert(p, allowed);
    }

    /// The settings rows of one app: each permission its manifest asks for, with its grant.
    pub fn for_app(
        &self,
        app_id: &str,
        requested: &[DevicePermission],
    ) -> Vec<(DevicePermission, bool)> {
        requested
            .iter()
            .map(|&p| (p, self.allowed(app_id, p)))
            .collect()
    }

    /// `{"<appId>": {"media": true, …}, …}` for IronTeams's config file.
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(app, m)| {
                    let perms: Map<String, Value> = m
                        .iter()
                        .map(|(p, ok)| (p.as_str().to_owned(), Value::Bool(*ok)))
                        .collect();
                    (app.clone(), Value::Object(perms))
                })
                .collect(),
        )
    }

    /// Reads [`Grants::to_json`]; unknown permissions and malformed entries are dropped.
    pub fn from_json(v: &Value) -> Self {
        let mut g = Grants::default();
        for (app, perms) in v.as_object().into_iter().flatten() {
            for (p, ok) in perms.as_object().into_iter().flatten() {
                if let (Some(p), Some(ok)) = (DevicePermission::parse(p), ok.as_bool()) {
                    g.set(app, p, ok);
                }
            }
        }
        g
    }
}
