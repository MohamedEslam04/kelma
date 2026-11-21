//! The app bar (left rail) and the "View more apps" flyout: the user's entitled apps.
//!
//! One read gives both: `POST beta/users/apps/aggregatedEntitlements?appbarview=userpinned`
//! with body `[]` answers every app the user is entitled to (`userEntitlements`, keyed by the
//! user id) with its definition (`definitions`, keyed by app id). Pinned apps carry
//! `isAppBarPinned` and `appBarOrder`; admin pins (setup policy) add `isAdminPinned`, user pins
//! `isUserPinned`.

use std::collections::HashMap;

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::graph::str_of;
use teams_proto::{Client, Result};

use crate::api;
use crate::install::Request;
use crate::model::AppDefinition;

/// How the user has an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntitlementState {
    Installed,
    /// Installed and cannot be removed (built-in and policy-installed apps).
    InstalledAndPermanent,
    InstalledAndDeprecated,
    /// Admin consented, not installed by the user.
    PreConsented,
    /// Suggested by Teams, not installed.
    Featured,
    Other(String),
}

impl EntitlementState {
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "Installed" => Self::Installed,
            "InstalledAndPermanent" => Self::InstalledAndPermanent,
            "InstalledAndDeprecated" => Self::InstalledAndDeprecated,
            "PreConsented" => Self::PreConsented,
            "Featured" => Self::Featured,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Installed and in use (not deprecated).
    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Installed | Self::InstalledAndPermanent)
    }
}

/// One entitled app of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlement {
    pub id: String,
    pub state: EntitlementState,
    pub pinned: bool,
    pub admin_pinned: bool,
    pub user_pinned: bool,
    /// Position on the app bar (1 is the top); gaps are normal.
    pub order: Option<u32>,
    pub installed_version: Option<String>,
}

impl Entitlement {
    fn parse(v: &Value) -> Option<Self> {
        let flag = |k: &str| v[k].as_bool().unwrap_or(false);
        Some(Entitlement {
            id: str_of(v, "id")?,
            state: EntitlementState::parse(v["state"].as_str().unwrap_or_default()),
            pinned: flag("isAppBarPinned"),
            admin_pinned: flag("isAdminPinned"),
            user_pinned: flag("isUserPinned"),
            order: v["appBarOrder"].as_u64().map(|o| o as u32),
            installed_version: str_of(v, "installedVersion"),
        })
    }

    /// An update is available ("Manage your apps"): the installed version differs from the
    /// current definition's.
    pub fn has_update(&self, def: Option<&AppDefinition>) -> bool {
        match (&self.installed_version, def) {
            (Some(v), Some(d)) => !d.version.is_empty() && *v != d.version,
            _ => false,
        }
    }
}

/// The user's entitled apps with their definitions.
#[derive(Debug, Clone, Default)]
pub struct InstalledApps {
    pub entitlements: Vec<Entitlement>,
    pub definitions: HashMap<String, AppDefinition>,
    /// `userEntitlementsHash`: changes whenever the entitlements change.
    pub hash: String,
}

/// Parses the aggregated entitlements answer.
pub fn parse_installed(v: &Value) -> InstalledApps {
    let value = &v["value"];
    let entitlements = value["userEntitlements"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .flat_map(|list| list.as_array().into_iter().flatten())
        .filter_map(Entitlement::parse)
        .collect();
    let definitions = value["definitions"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter_map(AppDefinition::parse)
        .map(|d| (d.id.clone(), d))
        .collect();
    InstalledApps {
        entitlements,
        definitions,
        hash: str_of(value, "userEntitlementsHash").unwrap_or_default(),
    }
}

/// The user's entitled apps (the app bar and the flyout).
pub async fn installed_apps(client: &Client) -> Result<InstalledApps> {
    let v = api::read(
        client,
        "app entitlements",
        Method::POST,
        "beta/users/apps/aggregatedEntitlements?appbarview=userpinned",
        Some(&json!([])),
    )
    .await?;
    Ok(parse_installed(&v))
}

impl InstalledApps {
    pub fn definition(&self, id: &str) -> Option<&AppDefinition> {
        self.definitions.get(id)
    }

    /// The pinned apps, top to bottom.
    pub fn app_bar(&self) -> Vec<(&Entitlement, &AppDefinition)> {
        let mut bar: Vec<_> = self
            .entitlements
            .iter()
            .filter(|e| e.pinned)
            .filter_map(|e| Some((e, self.definition(&e.id)?)))
            .collect();
        bar.sort_by_key(|(e, _)| e.order.unwrap_or(u32::MAX));
        bar
    }

    /// The "View more apps" flyout: installed apps with a personal tab or bot that are not
    /// pinned and not built in, by name, filtered by `query` (case-insensitive, empty for all).
    pub fn more_apps(&self, query: &str) -> Vec<&AppDefinition> {
        let q = query.to_lowercase();
        let mut apps: Vec<&AppDefinition> = self
            .entitlements
            .iter()
            .filter(|e| e.state.is_installed() && !e.pinned)
            .filter_map(|e| self.definition(&e.id))
            .filter(|d| d.has_personal_app && !d.is_core)
            .filter(|d| d.name.to_lowercase().contains(&q))
            .collect();
        apps.sort_by_key(|d| d.name.to_lowercase());
        apps.dedup_by(|a, b| a.id == b.id);
        apps
    }
}

/// A change to the app bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinChange {
    /// Pin at the bottom.
    Pin(String),
    Unpin(String),
    /// Move a pinned app to `index` (0 is the top).
    Move(String, usize),
}

/// The pin, unpin or reorder request (`pinUnpinApp`): the web client posts the whole new pinned
/// list as `[{appId, appBarOrder}]` to `beta/users/apps/pinned`. Taken from the web client's
/// code and never sent live: IronTeams only prints it (dry run).
pub fn pin_request(base: &str, apps: &InstalledApps, change: &PinChange) -> Request {
    let mut ids: Vec<String> = apps.app_bar().iter().map(|(e, _)| e.id.clone()).collect();
    match change {
        PinChange::Pin(id) => {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        PinChange::Unpin(id) => ids.retain(|i| i != id),
        PinChange::Move(id, to) => {
            if let Some(from) = ids.iter().position(|i| i == id) {
                let id = ids.remove(from);
                ids.insert((*to).min(ids.len()), id);
            }
        }
    }
    let body: Vec<Value> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| json!({"appId": id, "appBarOrder": i + 1}))
        .collect();
    Request {
        method: Method::POST,
        url: format!("{base}/beta/users/apps/pinned"),
        body: Some(Value::Array(body)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apps() -> InstalledApps {
        parse_installed(&json!({"value": {"userEntitlements": {"u": [
            {"id": "a", "state": "InstalledAndPermanent", "isAppBarPinned": true, "appBarOrder": 1},
            {"id": "b", "state": "Installed", "isAppBarPinned": true, "appBarOrder": 3},
            {"id": "c", "state": "Installed"}
        ]}, "definitions": {
            "a": {"id": "a", "name": "A"}, "b": {"id": "b", "name": "B"}, "c": {"id": "c", "name": "C"}
        }}}))
    }

    fn order(r: &Request) -> Vec<(String, u64)> {
        r.body
            .as_ref()
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["appId"].as_str().unwrap().to_owned(),
                    p["appBarOrder"].as_u64().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn pin_appends_and_renumbers() {
        let r = pin_request("https://mt", &apps(), &PinChange::Pin("c".into()));
        assert_eq!(r.method, Method::POST);
        assert_eq!(r.url, "https://mt/beta/users/apps/pinned");
        assert_eq!(
            order(&r),
            [("a".into(), 1), ("b".into(), 2), ("c".into(), 3)]
        );
        let again = pin_request("https://mt", &apps(), &PinChange::Pin("a".into()));
        assert_eq!(
            order(&again).len(),
            2,
            "pinning a pinned app changes nothing"
        );
    }

    #[test]
    fn unpin_and_move() {
        let r = pin_request("https://mt", &apps(), &PinChange::Unpin("a".into()));
        assert_eq!(order(&r), [("b".into(), 1)]);
        let r = pin_request("https://mt", &apps(), &PinChange::Move("b".into(), 0));
        assert_eq!(order(&r), [("b".into(), 1), ("a".into(), 2)]);
        let r = pin_request("https://mt", &apps(), &PinChange::Move("a".into(), 99));
        assert_eq!(order(&r), [("b".into(), 1), ("a".into(), 2)]);
    }

    #[test]
    fn states_and_updates() {
        assert!(EntitlementState::parse("Installed").is_installed());
        assert!(!EntitlementState::parse("Featured").is_installed());
        assert_eq!(
            EntitlementState::parse("Weird"),
            EntitlementState::Other("Weird".into())
        );
        let e = Entitlement::parse(&json!({"id": "x", "installedVersion": "1.0"})).unwrap();
        let d = AppDefinition::parse(&json!({"id": "x", "version": "1.0"})).unwrap();
        assert!(!e.has_update(Some(&d)));
        assert!(!e.has_update(None));
        assert!(Entitlement::parse(&json!({"state": "Installed"})).is_none());
    }
}
