//! The app bar on the rail and the "View more apps" flyout. Pinning, unpinning and reordering
//! change the bar here at once and are sent to Teams (`POST beta/users/apps/pinned`, the request
//! the web client sends, UNVERIFIED live). When Teams refuses, the bar keeps the change on this
//! computer and the banner says so.

use super::Apps;
use super::view::{Card, RailData, RailItem, letter, tile_color};
use crate::backend::Engine;
use app_store::{AppDefinition, InstalledApps, PinChange, pin_request};
use std::sync::Arc;

/// The apps kelma has a screen of its own for, in rail order.
const NATIVE: [&str; 5] = ["activity", "chat", "teams", "calendar", "onedrive"];

/// The kelma view of a built-in app that has a native screen (matched by name).
pub(super) fn native_key(def: &AppDefinition) -> Option<&'static str> {
    Some(match def.name.to_lowercase().as_str() {
        "activity" => "activity",
        // The unified entry stands for Chat and Teams, which kelma shows as two views.
        "chat" | "chat and channels" => "chat",
        "teams" => "teams",
        "calendar" => "calendar",
        "onedrive" => "onedrive",
        _ => return None,
    })
}

/// The kelma view an app opens in: one of the built-in screens, or `app` (an app page with a
/// link that opens it in Teams on the web).
pub(super) fn app_view(def: &AppDefinition) -> &'static str {
    if def.id == super::fixtures::PLANNER {
        return "planner";
    }
    match def.name.to_lowercase().as_str() {
        "planner" | "tasks" | "tasks by planner and to do" | "to do" => "planner",
        "workflows" => "workflows",
        "onenote" => "onenote",
        "lists" | "microsoft lists" => "lists",
        "meeting recaps" | "recap" => "recaps",
        _ => "app",
    }
}

/// What the rail and the flyout need from an app definition.
struct Listed {
    id: String,
    name: String,
    developer: String,
    blurb: String,
    accent: String,
    icon: String,
    view: &'static str,
    locked: bool,
}

impl Listed {
    fn of(d: &AppDefinition, locked: bool) -> Listed {
        Listed {
            id: d.id.clone(),
            name: d.name.clone(),
            developer: d.developer.clone(),
            blurb: d.short_description.clone(),
            accent: d.accent_color.clone(),
            icon: d.large_icon.clone(),
            view: app_view(d),
            locked,
        }
    }
}

impl Apps {
    /// Pinned apps that do not have a native view, in bar order.
    fn extras(&self) -> Vec<Listed> {
        let Some(i) = &self.installed else {
            return Vec::new();
        };
        let mut seen = Vec::new();
        i.app_bar()
            .into_iter()
            .filter(|(_, d)| native_key(d).is_none())
            .filter(|(e, _)| {
                let fresh = !seen.contains(&e.id);
                seen.push(e.id.clone());
                fresh
            })
            .map(|(e, d)| Listed::of(d, e.admin_pinned))
            .collect()
    }

    /// The ids of the pinned apps without a native view, in bar order.
    pub(super) fn extra_ids(&self) -> Vec<String> {
        self.extras().into_iter().map(|e| e.id).collect()
    }

    pub(super) fn rail_items(&mut self) -> Vec<RailItem> {
        self.extras()
            .into_iter()
            .map(|e| RailItem {
                letter: letter(&e.name),
                color: tile_color(&e.accent, &e.name),
                icon: self.image(&e.icon),
                id: e.id,
                name: e.name,
                view: e.view.to_owned(),
                locked: e.locked,
            })
            .collect()
    }

    pub(super) fn is_pinned(&self, id: &str) -> bool {
        self.installed
            .as_ref()
            .is_some_and(|i| i.entitlements.iter().any(|e| e.id == id && e.pinned))
    }

    pub(super) fn is_installed(&self, id: &str) -> bool {
        self.installed.as_ref().is_some_and(|i| {
            i.entitlements
                .iter()
                .any(|e| e.id == id && e.state.is_installed())
        })
    }

    /// The flyout list: installed apps that are not pinned, by name.
    fn flyout_cards(&mut self) -> Vec<Card> {
        let Some(i) = &self.installed else {
            return Vec::new();
        };
        let found: Vec<Listed> = i
            .more_apps(&self.flyout_query)
            .into_iter()
            .map(|d| Listed::of(d, false))
            .collect();
        found
            .into_iter()
            .map(|e| Card {
                letter: letter(&e.name),
                color: tile_color(&e.accent, &e.name),
                icon: self.image(&e.icon),
                installed: true,
                id: e.id,
                name: e.name,
                developer: e.developer,
                blurb: e.blurb,
                ..Default::default()
            })
            .collect()
    }

    pub(super) fn rail_data(&mut self) -> RailData {
        let items = self.rail_items();
        let flyout = if self.flyout {
            self.flyout_cards()
        } else {
            Vec::new()
        };
        RailData {
            items,
            collapsed: self.collapsed,
            active: self.active.clone(),
            flyout_open: self.flyout,
            flyout_query: self.flyout_query.clone(),
            flyout,
            flyout_loading: self.installed.is_none(),
        }
    }

    /// Makes `ids` the bar, in this order: they are pinned, every other app is not (an app the
    /// admin pinned stays).
    fn apply_order(&mut self, ids: &[String]) {
        let Some(i) = &mut self.installed else {
            return;
        };
        for e in &mut i.entitlements {
            match ids.iter().position(|id| *id == e.id) {
                Some(p) => {
                    e.pinned = true;
                    e.user_pinned = !e.admin_pinned;
                    e.order = Some(p as u32 + 1);
                }
                None if !e.admin_pinned => {
                    e.pinned = false;
                    e.user_pinned = false;
                    e.order = None;
                }
                None => {}
            }
        }
    }
}

/// The ids a pin request orders, from its body.
fn request_ids(req: &app_store::Request) -> Vec<String> {
    req.body
        .as_ref()
        .and_then(|b| b.as_array())
        .into_iter()
        .flatten()
        .filter_map(|e| e["appId"].as_str().map(str::to_owned))
        .collect()
}

fn installed_name(i: &InstalledApps, id: &str) -> String {
    i.definition(id)
        .map_or_else(|| id.to_owned(), |d| d.name.clone())
}

impl Engine {
    pub(super) async fn apps_load(self: &Arc<Self>) -> anyhow::Result<()> {
        let installed = self.src_installed().await?;
        let policy = self.src_policy().await.ok();
        {
            let mut a = self.apps.lock().unwrap();
            a.installed = Some(installed);
            a.policy = policy;
        }
        self.apps_push_rail();
        Ok(())
    }

    /// Loads the installed apps when they are not loaded yet.
    pub(super) async fn apps_ensure_installed(self: &Arc<Self>) {
        if self.apps.lock().unwrap().installed.is_none()
            && let Err(err) = self.apps_load().await
        {
            tracing::warn!(%err, "installed apps load failed");
            self.ui.banner(format!("Could not load your apps: {err}"));
        }
    }

    pub(super) fn apps_push_rail(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().rail_data();
        self.ui.apps_rail(data);
        self.apps_fetch_images();
    }

    /// Applies a pin change to the bar here and sends it to Teams.
    async fn apps_pin_change(self: &Arc<Self>, change: PinChange) {
        let base = self.apps_base().await;
        let outcome = {
            let mut a = self.apps.lock().unwrap();
            let allowed = a.policy.is_none_or(|p| p.user_pinning_allowed);
            let Some(i) = &a.installed else {
                return;
            };
            let name = |id: &str| installed_name(i, id);
            let (id, what) = match &change {
                PinChange::Pin(id) => (id, format!("Pinned {}", name(id))),
                PinChange::Unpin(id) => (id, format!("Unpinned {}", name(id))),
                PinChange::Move(id, _) => (id, format!("Moved {}", name(id))),
            };
            let entitlement = i.entitlements.iter().find(|e| e.id == *id);
            if !allowed {
                Err("Your admin does not allow changing the app bar.".to_owned())
            } else if matches!(change, PinChange::Unpin(_))
                && entitlement.is_some_and(|e| e.admin_pinned)
            {
                Err(format!("{} is pinned by your admin.", name(id)))
            } else if matches!(change, PinChange::Pin(_))
                && !entitlement.is_some_and(|e| e.state.is_installed())
            {
                Err(format!("{} is not installed.", name(id)))
            } else {
                let req = pin_request(&base, i, &change);
                let ids = request_ids(&req);
                a.apply_order(&ids);
                Ok((req, what))
            }
        };
        match outcome {
            Ok((req, what)) => match self.src_send("app bar", req).await {
                Ok(()) => self.ui.banner(format!("{what}.")),
                Err(err) => {
                    tracing::warn!(%err, "app bar change not accepted");
                    self.ui.banner(format!(
                        "{what} on this computer only. Teams did not accept the change: {err}"
                    ));
                }
            },
            Err(msg) => self.ui.banner(msg),
        }
        self.apps_push_rail();
        self.shop_push();
    }

    pub(super) async fn apps_pin(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        self.apps_ensure_installed().await;
        self.apps_pin_change(PinChange::Pin(id.to_owned())).await;
        Ok(())
    }

    pub(super) async fn apps_unpin(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        self.apps_pin_change(PinChange::Unpin(id.to_owned())).await;
        Ok(())
    }

    /// Moves a pinned app to `to` among the pinned apps the rail lists. The request orders the
    /// whole bar, including the apps kelma shows itself, so the position is translated.
    pub(super) async fn apps_move(self: &Arc<Self>, id: &str, to: usize) -> anyhow::Result<()> {
        let full = {
            let a = self.apps.lock().unwrap();
            let Some(i) = &a.installed else {
                return Ok(());
            };
            let extras = a.extra_ids();
            let target = extras.get(to).or(extras.last()).cloned();
            let bar: Vec<String> = i.app_bar().iter().map(|(e, _)| e.id.clone()).collect();
            target.and_then(|t| bar.iter().position(|b| *b == t))
        };
        if let Some(full) = full {
            self.apps_pin_change(PinChange::Move(id.to_owned(), full))
                .await;
        }
        Ok(())
    }

    /// Opens a native view by its key (what the rail does for its fixed entries).
    pub(super) async fn apps_open_native(self: &Arc<Self>, key: &str) -> anyhow::Result<()> {
        self.ui.open_view(key.to_owned());
        match key {
            "activity" => self.activity_refresh().await?,
            "onedrive" => {
                self.drive_cmd(super::super::DriveCmd::Enter).await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Ctrl+Shift+N: the nth entry of the rail.
    pub(super) async fn apps_shortcut(self: &Arc<Self>, n: usize) -> anyhow::Result<()> {
        if n == 0 {
            return Ok(());
        }
        if let Some(key) = NATIVE.get(n - 1) {
            return self.apps_open_native(key).await;
        }
        let id = self
            .apps
            .lock()
            .unwrap()
            .extra_ids()
            .get(n - 1 - NATIVE.len())
            .cloned();
        match id {
            Some(id) => self.apps_open(&id).await,
            None => Ok(()),
        }
    }

    pub(super) async fn apps_flyout(
        self: &Arc<Self>,
        open: bool,
        query: Option<String>,
    ) -> anyhow::Result<()> {
        if open {
            self.apps_ensure_installed().await;
        }
        {
            let mut a = self.apps.lock().unwrap();
            a.flyout = open;
            match query {
                Some(q) => a.flyout_query = q,
                None if !open => a.flyout_query.clear(),
                None => {}
            }
        }
        self.apps_push_rail();
        Ok(())
    }

    /// Opens an installed app in the main area.
    pub(super) async fn apps_open(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        self.apps_ensure_installed().await;
        let (key, view) = {
            let mut a = self.apps.lock().unwrap();
            a.flyout = false;
            a.flyout_query.clear();
            let Some(def) = a.def(id) else {
                drop(a);
                self.apps_push_rail();
                return Ok(());
            };
            let found = (native_key(def), app_view(def));
            if found.0.is_none() {
                a.active = id.to_owned();
                a.shop.host = Some(id.to_owned());
            }
            found
        };
        self.apps_push_rail();
        if let Some(key) = key {
            return self.apps_open_native(key).await;
        }
        self.ui.open_view(view.to_owned());
        match view {
            "planner" => self.tasks_enter().await?,
            "workflows" => self.flows_enter().await?,
            "onenote" => self.notes_enter().await?,
            "lists" => self.lists_enter().await?,
            "recaps" => self.recaps_enter().await?,
            _ => self.shop_push(),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_store::parse_installed;
    use serde_json::json;

    fn def(name: &str) -> AppDefinition {
        AppDefinition::parse(&json!({"id": name, "name": name})).unwrap()
    }

    #[test]
    fn native_apps_and_views_are_matched_by_name() {
        assert_eq!(native_key(&def("Calendar")), Some("calendar"));
        assert_eq!(native_key(&def("Chat and Channels")), Some("chat"));
        assert_eq!(native_key(&def("Calling")), None);
        assert_eq!(app_view(&def("Tasks by Planner and To Do")), "planner");
        assert_eq!(app_view(&def("Workflows")), "workflows");
        assert_eq!(app_view(&def("Copilot")), "app");
    }

    #[test]
    fn the_rail_lists_pinned_apps_that_have_no_native_view_in_bar_order() {
        let installed = parse_installed(&super::super::fixtures::installed());
        let mut a = Apps {
            installed: Some(installed),
            ..Default::default()
        };
        let names: Vec<String> = a.rail_items().into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["Calling", "Planner", "Workflows"]);
        a.apply_order(&[
            "demo-workflows".into(),
            "com.microsoft.teamspace.tab.planner".into(),
        ]);
        let names: Vec<String> = a.rail_items().into_iter().map(|r| r.name).collect();
        // Calling was pinned by the admin and stays; the others follow the new order.
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"Calling".to_owned()));
        let pos = |n: &str| names.iter().position(|x| x == n).unwrap();
        assert!(pos("Workflows") < pos("Planner"));
    }
}
