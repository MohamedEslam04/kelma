//! Opening an app's tab in the web host (plan S3.5) instead of the system browser.
//!
//! The routing is pure: [`plan_tab`] says whether a tab of an app is a W tab and builds the
//! [`webhost::ShimConfig`] from the manifest; [`grants_from_text`] reads `app-grants.json`.
//! [`Engine::apps_open_in_host`] is the only part that launches anything, and in demo mode and
//! in tests it only records the target, like [`crate::external::open`].
//!
//! Routing rules: a tab opens in the host when its app has no kelma screen of its own, the
//! tab is a personal tab with an https `contentUrl`, and the URL is still https once its
//! placeholders are filled. Everything else (apps with a native screen, tabs without a web
//! URL, the plain "Open in Teams on the web" link, privacy and terms links) goes to the browser.

use super::Engine;
use super::bar::{app_view, native_key};
use app_store::{AppDefinition, DevicePermission, Grants};
use std::sync::Arc;
use webhost::{HostContext, ShimConfig};

/// Gets the result a task module's page submitted. Runs on the host window's task.
pub(crate) type TaskSubmit = Arc<dyn Fn(serde_json::Value) + Send + Sync>;

/// A tab ready to open in the host.
#[derive(Debug, Clone)]
pub(super) struct WebTarget {
    /// The `contentUrl` with its placeholders filled.
    pub url: String,
    /// The window title: the app's name.
    pub title: String,
    pub shim: ShimConfig,
    /// The device permissions the app asks for and the user granted in Store settings.
    pub granted: Vec<DevicePermission>,
}

/// Who the host tells the app it is working for. Only what the shim reports from `getContext`.
#[derive(Debug, Clone, Default)]
pub(super) struct Who {
    pub display_name: Option<String>,
    /// The signed-in user's MRI (`8:orgid:<object id>`).
    pub self_mri: Option<String>,
    /// A tenant id, or `organizations` while none is chosen.
    pub tenant: String,
}

/// Whether the app's tabs belong in the host: it has no screen of its own in kelma.
pub(super) fn is_web_app(def: &AppDefinition) -> bool {
    native_key(def).is_none() && app_view(def) == "app"
}

/// `webApplicationInfo.resource` of the manifest (`api://<domain>/<app id>`), if it has one.
pub(super) fn sso_resource(def: &AppDefinition) -> Option<String> {
    def.raw["webApplicationInfo"]["resource"]
        .as_str()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// The host context for one tab of an app.
fn context(def: &AppDefinition, entity_id: &str, who: &Who) -> HostContext {
    let mut c = HostContext {
        app_id: def.id.clone(),
        entity_id: entity_id.to_owned(),
        ..HostContext::default()
    };
    c.user_display_name = who.display_name.clone().unwrap_or_default();
    if let Some(mri) = &who.self_mri {
        c.user_object_id = mri.trim_start_matches("8:orgid:").to_owned();
    }
    if who.tenant != "organizations" {
        c.tenant_id = who.tenant.clone();
    }
    c
}

/// The web host target for the personal tab `entity_id` of `def`, or `None` when the tab opens
/// in the browser (see the module docs).
pub(super) fn plan_tab(
    def: &AppDefinition,
    entity_id: &str,
    who: &Who,
    grants: &Grants,
) -> Option<WebTarget> {
    if !is_web_app(def) {
        return None;
    }
    let tab = def.personal_tabs().find(|t| t.entity_id == entity_id)?;
    target(
        def,
        entity_id,
        tab.content_url.as_deref()?,
        &def.name,
        who,
        grants,
    )
}

/// The web host target for the task module of a composer action command (plan S3.6): `url` is
/// the command's `taskInfo.url` or the one `composeExtension/fetchTask` answered, `command_id`
/// stands in for the entity id. The shim is built like a tab's, from the app's manifest. `None`
/// (the caller opens nothing) when the app is unknown or the URL is not https.
pub(super) fn plan_task(
    def: Option<&AppDefinition>,
    command_id: &str,
    url: &str,
    title: &str,
    who: &Who,
    grants: &Grants,
) -> Option<WebTarget> {
    let def = def?;
    let title = if title.trim().is_empty() {
        &def.name
    } else {
        title.trim()
    };
    target(def, command_id, url, title, who, grants)
}

/// A page of `def` at `template` (placeholders not yet filled), if it is https.
fn target(
    def: &AppDefinition,
    entity_id: &str,
    template: &str,
    title: &str,
    who: &Who,
    grants: &Grants,
) -> Option<WebTarget> {
    if !template.starts_with("https://") {
        return None;
    }
    let ctx = context(def, entity_id, who);
    let url = webhost::shim::expand_url(template, &ctx);
    if !url.starts_with("https://") || webhost::shim::origin_of(&url).is_none() {
        return None;
    }
    let granted: Vec<DevicePermission> = def
        .device_permissions
        .iter()
        .copied()
        .filter(|p| grants.allowed(&def.id, *p))
        .collect();
    // The shim refuses everything not in this list, with the manifest spelling as the name.
    let shim = ShimConfig::for_app(
        ctx,
        &url,
        sso_resource(def),
        def.valid_domains.clone(),
        granted.iter().map(|p| p.as_str().to_owned()).collect(),
    );
    Some(WebTarget {
        url,
        title: title.to_owned(),
        shim,
        granted,
    })
}

/// Reads the text of `app-grants.json`. A missing, unreadable or malformed file grants nothing.
pub(super) fn grants_from_text(text: Option<&str>) -> Grants {
    text.and_then(|t| serde_json::from_str(t).ok())
        .map(|v| Grants::from_json(&v))
        .unwrap_or_default()
}

impl Engine {
    /// Opens the personal tab `entity_id` of the app page that is showing in the web host.
    /// Returns `false` when the tab is not a W tab; the caller then uses the browser.
    pub(super) fn apps_open_in_host(self: &Arc<Self>, entity_id: &str) -> bool {
        let who = self.apps_who();
        let planned = {
            let a = self.apps.lock().unwrap();
            let Some(def) = a.shop.host.as_ref().and_then(|h| a.def(h)) else {
                return false;
            };
            let grants = if self.apps_offline() {
                Grants::default()
            } else {
                grants_from_text(super::shop::read_grants_file().as_deref())
            };
            plan_tab(def, entity_id, &who, &grants).map(|t| (t, def.open_link(entity_id)))
        };
        let Some((target, fallback)) = planned else {
            return false;
        };
        self.apps
            .lock()
            .unwrap()
            .opened
            .push(format!("web-host:{}", target.url));
        if self.apps_offline() || crate::external::suppressed() {
            return true;
        }
        tracing::debug!(app = %target.shim.context.app_id, granted = ?target.granted, "opening in the web host");
        let e = self.clone();
        tokio::spawn(async move {
            if let Err(err) = e.run_host_window(target, None).await {
                tracing::warn!(%err, "web host did not open");
                e.ui.banner(format!("Could not open the app in the web host: {err}"));
                if matches!(err, webhost::Error::NoBrowser) {
                    e.apps_open_link(&fallback);
                }
            }
        });
        true
    }

    /// Opens the task module of a composer action command (plan S3.6) in the web host. `def` is
    /// the command's app; `on_submit` gets the result the page submits (`tasks.submitTask`,
    /// `dialog.url.submit`), after which the window closes. Returns `false`, and opens nothing
    /// (no browser either), when the app is unknown or `url` is not an https link. Demo mode
    /// and tests only record `web-host:<url>`.
    pub(crate) fn apps_open_task(
        self: &Arc<Self>,
        def: Option<&AppDefinition>,
        command_id: &str,
        url: &str,
        title: &str,
        on_submit: TaskSubmit,
    ) -> bool {
        let grants = if self.apps_offline() {
            Grants::default()
        } else {
            grants_from_text(super::shop::read_grants_file().as_deref())
        };
        let Some(target) = plan_task(def, command_id, url, title, &self.apps_who(), &grants) else {
            return false;
        };
        self.apps
            .lock()
            .unwrap()
            .opened
            .push(format!("web-host:{}", target.url));
        if self.apps_offline() || crate::external::suppressed() {
            // Tests play the page's submit through this.
            #[cfg(test)]
            {
                self.apps.lock().unwrap().task_submit = Some(on_submit);
            }
            return true;
        }
        tracing::debug!(app = %target.shim.context.app_id, granted = ?target.granted, "opening a task module in the web host");
        let e = self.clone();
        tokio::spawn(async move {
            if let Err(err) = e.run_host_window(target, Some(on_submit)).await {
                tracing::warn!(%err, "web host did not open");
                e.ui.banner(format!("Could not open the form in the web host: {err}"));
            }
        });
        true
    }

    /// What a task module's page would call when it submits (tests).
    #[cfg(test)]
    pub(crate) fn apps_task_submit(&self) -> Option<TaskSubmit> {
        self.apps.lock().unwrap().task_submit.clone()
    }

    /// Who the host tells an app it is working for.
    fn apps_who(&self) -> Who {
        let cfg = self.cfg.lock().unwrap();
        Who {
            display_name: cfg.display_name.clone(),
            self_mri: cfg.self_mri.clone(),
            tenant: cfg.tenant.clone(),
        }
    }

    /// The definition of an installed or Store app, if the app pages loaded it.
    pub(crate) fn apps_definition(&self, app_id: &str) -> Option<AppDefinition> {
        self.apps.lock().unwrap().def(app_id).cloned()
    }

    /// Opens the window and keeps it until the user closes it. With `on_submit`, the first
    /// result the page submits goes to it and closes the window.
    async fn run_host_window(
        self: &Arc<Self>,
        target: WebTarget,
        on_submit: Option<TaskSubmit>,
    ) -> webhost::Result<()> {
        let paths = session::Paths::new().map_err(|e| webhost::Error::Startup(e.to_string()))?;
        let mut opts = webhost::WebOptions::new(webhost::default_profile_dir(&paths.data_dir));
        opts.title = Some(target.title.clone());
        opts.size = Some((1200, 800));
        opts.shim = Some(target.shim.clone());
        let client = self.client.clone();
        opts.token = Some(Arc::new(move |req: webhost::TokenRequest| {
            let client = client.clone();
            Box::pin(async move {
                let resource = req.resources.first().ok_or("no resource")?.clone();
                let claims = webhost::shim::merge_claims(&req.claims);
                client
                    .app_token(&resource, claims.as_deref())
                    .await
                    .map(|t| t.access_token)
                    .map_err(|e| webhost::shim::token_failure_reason(&e.to_string()))
            })
        }));
        let win = webhost::open(&target.url, opts).await?;
        let Some(on_submit) = on_submit else {
            win.closed().await;
            return Ok(());
        };
        let mut events = win.events();
        loop {
            tokio::select! {
                _ = win.closed() => break,
                event = events.recv() => match event {
                    Ok(webhost::AppEvent::TaskSubmit(result)) => {
                        on_submit(result);
                        win.close().await;
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                },
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// A fake manifest as the app service sends it.
    fn manifest(extra: Value) -> AppDefinition {
        let mut v = json!({
            "id": "fake-app-1",
            "name": "Fake Tickets",
            "version": "1.0.0",
            "validDomains": ["tabs.example.invalid", "*.example.invalid"],
            "staticTabs": [
                {"entityId": "home", "name": "Home",
                 "contentUrl": "https://tabs.example.invalid/home?l={locale}&t={tid}&e={entityId}"},
                {"entityId": "about"},
                {"entityId": "plain", "name": "Plain",
                 "contentUrl": "http://tabs.example.invalid/plain"},
            ],
            "metaCapabilities": {"staticTabs": {"scopes": ["Personal"]}},
            "devicePermissions": ["media", "geolocation"],
        });
        for (k, val) in extra.as_object().into_iter().flatten() {
            v[k] = val.clone();
        }
        AppDefinition::parse(&v).unwrap()
    }

    fn who() -> Who {
        Who {
            display_name: Some("Test User".into()),
            self_mri: Some("8:orgid:00000000-0000-0000-0000-000000000009".into()),
            tenant: "00000000-0000-0000-0000-0000000000aa".into(),
        }
    }

    #[test]
    fn an_app_with_sso_gets_its_resource_domains_and_a_filled_url() {
        let def = manifest(json!({"webApplicationInfo": {
            "id": "0d5e", "resource": "api://tabs.example.invalid/0d5e"}}));
        let t = plan_tab(&def, "home", &who(), &Grants::default()).unwrap();
        assert_eq!(
            t.url,
            "https://tabs.example.invalid/home?l=en-us&t=00000000-0000-0000-0000-0000000000aa&e=home"
        );
        assert_eq!(t.title, "Fake Tickets");
        assert_eq!(
            t.shim.sso_resource.as_deref(),
            Some("api://tabs.example.invalid/0d5e")
        );
        assert_eq!(
            t.shim.valid_domains,
            ["tabs.example.invalid", "*.example.invalid"]
        );
        // The tab origin once, the wildcard is not an origin.
        assert_eq!(t.shim.allowed_origins, ["https://tabs.example.invalid"]);
        let c = &t.shim.context;
        assert_eq!(c.app_id, "fake-app-1");
        assert_eq!(c.entity_id, "home");
        assert_eq!(c.user_object_id, "00000000-0000-0000-0000-000000000009");
        assert_eq!(c.tenant_id, "00000000-0000-0000-0000-0000000000aa");
        assert_eq!(c.user_display_name, "Test User");
        // The resource passes the host's own check for the tab's page.
        let req = webhost::TokenRequest {
            resources: vec![],
            claims: vec![],
            silent: true,
            tenant_id: None,
            origin: "https://tabs.example.invalid".into(),
        };
        assert!(webhost::shim::check_token_request(&t.shim, req).is_ok());
    }

    #[test]
    fn an_app_without_web_application_info_has_no_sso_resource() {
        let def = manifest(json!({}));
        assert_eq!(sso_resource(&def), None);
        let t = plan_tab(&def, "home", &who(), &Grants::default()).unwrap();
        assert_eq!(t.shim.sso_resource, None);
        // With no resource the shim refuses a token request that names none.
        let req = webhost::TokenRequest {
            resources: vec![],
            claims: vec![],
            silent: true,
            tenant_id: None,
            origin: "https://tabs.example.invalid".into(),
        };
        assert!(webhost::shim::check_token_request(&t.shim, req).is_err());
        let blank = manifest(json!({"webApplicationInfo": {"resource": "  "}}));
        assert_eq!(sso_resource(&blank), None);
    }

    #[test]
    fn valid_domain_wildcards_widen_only_the_token_check() {
        let def = manifest(json!({
            "validDomains": ["*.example.invalid", "api.example.invalid"],
            "webApplicationInfo": {"resource": "api://api.example.invalid/0d5e"},
        }));
        let t = plan_tab(&def, "home", &who(), &Grants::default()).unwrap();
        assert_eq!(
            t.shim.allowed_origins,
            [
                "https://tabs.example.invalid",
                "https://api.example.invalid"
            ]
        );
        // The resource names api.example.invalid, not the page's host: a valid domain allows it.
        let req = webhost::TokenRequest {
            resources: vec![],
            claims: vec![],
            silent: true,
            tenant_id: None,
            origin: "https://tabs.example.invalid".into(),
        };
        assert!(webhost::shim::check_token_request(&t.shim, req).is_ok());
        // A lookalike is not covered by `*.example.invalid`.
        let evil = manifest(json!({
            "validDomains": ["*.example.invalid"],
            "webApplicationInfo": {"resource": "api://a.b.example.invalid/0d5e"},
        }));
        let t = plan_tab(&evil, "home", &who(), &Grants::default()).unwrap();
        let req = webhost::TokenRequest {
            resources: vec![],
            claims: vec![],
            silent: true,
            tenant_id: None,
            origin: "https://tabs.example.invalid".into(),
        };
        assert!(webhost::shim::check_token_request(&t.shim, req).is_err());
    }

    #[test]
    fn only_https_personal_tabs_of_apps_without_a_screen_go_to_the_host() {
        let def = manifest(json!({}));
        assert!(plan_tab(&def, "home", &who(), &Grants::default()).is_some());
        // The about tab has no URL, the plain tab is http, an unknown tab does not exist.
        assert!(plan_tab(&def, "about", &who(), &Grants::default()).is_none());
        assert!(plan_tab(&def, "plain", &who(), &Grants::default()).is_none());
        assert!(plan_tab(&def, "nope", &who(), &Grants::default()).is_none());
        // Apps kelma has a screen for stay there.
        for name in [
            "OneNote",
            "Workflows",
            "Lists",
            "Planner",
            "Activity",
            "Calendar",
        ] {
            let native = manifest(json!({"name": name}));
            assert!(!is_web_app(&native), "{name}");
            assert!(plan_tab(&native, "home", &who(), &Grants::default()).is_none());
        }
        // A placeholder cannot turn the URL into another scheme or host.
        let sneaky = manifest(json!({"staticTabs": [
            {"entityId": "home", "contentUrl": "https://{loginHint}.example.invalid/x"},
            {"entityId": "p2", "contentUrl": "{theme}://x.example.invalid/"},
        ]}));
        assert!(plan_tab(&sneaky, "p2", &who(), &Grants::default()).is_none());
    }

    #[test]
    fn a_missing_grants_file_grants_nothing_and_a_file_grants_what_it_says() {
        let def = manifest(json!({}));
        let none = grants_from_text(None);
        let t = plan_tab(&def, "home", &who(), &none).unwrap();
        assert!(t.granted.is_empty());
        assert!(!grants_from_text(Some("not json")).allowed("fake-app-1", DevicePermission::Media));

        let text = r#"{"fake-app-1": {"media": true, "geolocation": false, "midi": true},
                       "other-app": {"geolocation": true}}"#;
        let g = grants_from_text(Some(text));
        let t = plan_tab(&def, "home", &who(), &g).unwrap();
        // Only permissions the manifest asks for and the user allowed.
        assert_eq!(t.granted, [DevicePermission::Media]);
        // The shim gets the same list, so it refuses the rest.
        assert_eq!(t.shim.granted, ["media"]);
        let none = plan_tab(&def, "home", &who(), &Grants::default()).unwrap();
        assert!(none.shim.granted.is_empty());
    }

    #[test]
    fn an_action_commands_task_module_gets_the_apps_shim_config() {
        let def = manifest(json!({"webApplicationInfo": {
            "id": "0d5e", "resource": "api://tabs.example.invalid/0d5e"}}));
        let text = r#"{"fake-app-1": {"media": true, "geolocation": false}}"#;
        let g = grants_from_text(Some(text));
        let t = plan_task(
            Some(&def),
            "createTicket",
            "https://tabs.example.invalid/new?t={tid}&e={entityId}",
            "New ticket",
            &who(),
            &g,
        )
        .unwrap();
        assert_eq!(
            t.url,
            "https://tabs.example.invalid/new?t=00000000-0000-0000-0000-0000000000aa&e=createTicket"
        );
        assert_eq!(t.title, "New ticket");
        assert_eq!(
            t.shim.sso_resource.as_deref(),
            Some("api://tabs.example.invalid/0d5e")
        );
        assert_eq!(
            t.shim.valid_domains,
            ["tabs.example.invalid", "*.example.invalid"]
        );
        assert_eq!(t.shim.allowed_origins, ["https://tabs.example.invalid"]);
        assert_eq!(t.shim.context.app_id, "fake-app-1");
        assert_eq!(t.shim.context.entity_id, "createTicket");
        // Only what the manifest asks for and the user granted, in the shim too.
        assert_eq!(t.granted, [DevicePermission::Media]);
        assert_eq!(t.shim.granted, ["media"]);
        // A blank title falls back to the app's name.
        let t = plan_task(
            Some(&def),
            "c",
            "https://tabs.example.invalid/new",
            "  ",
            &who(),
            &g,
        )
        .unwrap();
        assert_eq!(t.title, "Fake Tickets");
    }

    #[test]
    fn a_task_module_that_is_not_an_https_page_of_a_known_app_opens_nowhere() {
        let def = manifest(json!({}));
        let g = Grants::default();
        let plan = |def: Option<&AppDefinition>, url: &str| {
            plan_task(def, "c", url, "T", &who(), &g).is_some()
        };
        assert!(plan(Some(&def), "https://tabs.example.invalid/new"));
        // Unknown app: no shim config can be built.
        assert!(!plan(None, "https://tabs.example.invalid/new"));
        // Not https: http, other schemes, relative and empty links, and a placeholder that
        // turns the link into another scheme.
        for url in [
            "http://tabs.example.invalid/new",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "/new",
            "",
            "{theme}://x.example.invalid/",
        ] {
            assert!(!plan(Some(&def), url), "{url}");
        }
        // An app with a screen of its own in kelma still gets its task module in the host:
        // only its tabs stay native.
        let native = manifest(json!({"name": "Planner"}));
        assert!(plan(Some(&native), "https://tabs.example.invalid/new"));
    }

    #[test]
    fn the_tenant_is_left_out_while_none_is_chosen() {
        let def = manifest(json!({}));
        let w = Who {
            tenant: "organizations".into(),
            ..Who::default()
        };
        let t = plan_tab(&def, "home", &w, &Grants::default()).unwrap();
        assert_eq!(t.shim.context.tenant_id, "");
        assert_eq!(t.shim.context.user_object_id, "");
    }

    #[test]
    fn grants_written_by_settings_are_what_the_host_reads() {
        let dir = std::env::temp_dir().join(format!("kelma-grants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut local = crate::backend::settings::Local::load(&dir);
        local.grants.set("app-1", DevicePermission::Media, true);
        local.save_grants().unwrap();
        let text = std::fs::read_to_string(dir.join("app-grants.json")).unwrap();
        let g = grants_from_text(Some(&text));
        assert!(g.allowed("app-1", DevicePermission::Media));
        assert!(!g.allowed("app-1", DevicePermission::Midi));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
