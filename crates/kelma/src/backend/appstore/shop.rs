//! The Store: home, search, categories, the app page, Manage your apps and the Store settings.
//! Add, remove and update only show the request Teams would get; kelma never sends it.

use super::Apps;
use super::view::{
    AppPage, Card, Nav, PermItem, StoreData, cert_text, letter, price_text, tile_color, works_text,
};
use crate::backend::Engine;
use app_store::{
    AppDefinition, AppDetails, AppPolicy, CatalogApp, DevicePermission, Filter, Price, Rating,
    Scope, SearchHit, StoreEntry, StoreHome, Target, WorksIn, YourApp, install_request,
    uninstall_request, update_request,
};
use std::sync::Arc;

/// "Devices" in the Store links out to the certified hardware list.
const DEVICES_URL: &str = "https://www.microsoft.com/microsoft-teams/across-devices/devices";
/// How many apps of a category are shown (their definitions are fetched together).
const PAGE_SIZE: usize = 60;
/// How many apps the home strips show.
const STRIP: usize = 12;

#[derive(Default)]
pub struct Shop {
    page: String,
    home: Option<StoreHome>,
    org: Vec<CatalogApp>,
    hits: Vec<SearchHit>,
    /// A category key, or `org` for "Built for your org".
    category: String,
    title: String,
    scope: Option<Scope>,
    agents: Option<bool>,
    details: Option<AppDetails>,
    mine: Vec<YourApp>,
    status: String,
    loading: bool,
    /// The app whose page kelma shows through the browser.
    pub host: Option<String>,
}

/// What a card needs from a definition or a search hit.
struct Info {
    id: String,
    name: String,
    developer: String,
    blurb: String,
    accent: String,
    icon: String,
    works: WorksIn,
    agent: bool,
    version: String,
    rating: Option<Rating>,
    price: Price,
}

impl Info {
    fn of_def(d: &AppDefinition, entry: Option<&StoreEntry>) -> Info {
        Info {
            id: d.id.clone(),
            name: d.name.clone(),
            developer: d.developer.clone(),
            blurb: d.short_description.clone(),
            accent: d.accent_color.clone(),
            icon: d.large_icon.clone(),
            works: d.works_in,
            agent: d.is_agent,
            version: d.version.clone(),
            rating: entry.and_then(|e| e.rating),
            price: Price::Unknown,
        }
    }

    fn of_hit(h: &SearchHit, entry: Option<&StoreEntry>) -> Info {
        Info {
            id: h.id.clone(),
            name: h.name.clone(),
            developer: h.developer.clone(),
            blurb: h.short_description.clone(),
            accent: h.accent_color.clone(),
            icon: h.icon.clone(),
            works: h.works_in,
            agent: h.is_agent,
            version: h.version.clone(),
            rating: entry.and_then(|e| e.rating),
            price: h.price,
        }
    }
}

/// `popularApps` as `Popular apps`.
pub(super) fn category_label(key: &str) -> String {
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub(super) fn permission_label(p: DevicePermission) -> &'static str {
    match p {
        DevicePermission::Media => "Camera and microphone",
        DevicePermission::Geolocation => "Location",
        DevicePermission::Notifications => "Notifications",
        DevicePermission::Midi => "MIDI devices",
        DevicePermission::OpenExternal => "Open external links",
    }
}

fn parse_scope(s: &str) -> Option<Scope> {
    match s {
        "personal" => Some(Scope::Personal),
        "chat" => Some(Scope::Chat),
        "team" => Some(Scope::Team),
        "meeting" => Some(Scope::Meeting),
        _ => None,
    }
}

impl Shop {
    fn filter(&self) -> Filter {
        Filter {
            scope: self.scope,
            agents: self.agents,
        }
    }

    fn entry(&self, id: &str) -> Option<&StoreEntry> {
        self.home
            .iter()
            .flat_map(|h| h.categories.iter())
            .flat_map(|c| c.apps.iter())
            .find(|e| e.id == id)
    }

    /// The ids of a category's apps that are not blocked.
    fn category_ids(&self, key: &str, limit: usize) -> Vec<String> {
        self.home
            .as_ref()
            .and_then(|h| h.category(key))
            .map(|c| {
                c.apps
                    .iter()
                    .filter(|e| !e.is_blocked)
                    .take(limit)
                    .map(|e| e.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Apps {
    fn has_def(&self, id: &str) -> bool {
        self.def(id).is_some()
    }

    fn cards(&mut self, infos: Vec<Info>) -> Vec<Card> {
        infos
            .into_iter()
            .map(|i| {
                let installed = self.is_installed(&i.id);
                let update = installed
                    && self.installed.as_ref().is_some_and(|inst| {
                        inst.entitlements.iter().any(|e| {
                            e.id == i.id
                                && e.installed_version
                                    .as_deref()
                                    .is_some_and(|v| !i.version.is_empty() && v != i.version)
                        })
                    });
                let mut meta = Vec::new();
                if let Some(r) = i.rating {
                    meta.push(format!("★ {:.1}", r.average));
                }
                meta.push(price_text(i.price).to_owned());
                meta.push(works_text(i.works));
                meta.retain(|m| !m.is_empty());
                Card {
                    letter: letter(&i.name),
                    color: tile_color(&i.accent, &i.name),
                    icon: self.image(&i.icon),
                    pinned: self.is_pinned(&i.id),
                    meta: meta.join(" · "),
                    installed,
                    update,
                    agent: i.agent,
                    id: i.id,
                    name: i.name,
                    developer: i.developer,
                    blurb: i.blurb,
                }
            })
            .collect()
    }

    /// Cards for app ids whose definitions are known, filtered by works-in and app or agent.
    fn id_cards(&mut self, ids: &[String]) -> Vec<Card> {
        let filter = self.shop.filter();
        let infos: Vec<Info> = ids
            .iter()
            .filter_map(|id| {
                let d = self.def(id)?;
                Some(Info::of_def(d, self.shop.entry(id)))
            })
            .filter(|i| filter.matches(i.works, i.agent))
            .collect();
        self.cards(infos)
    }

    fn app_page(&mut self) -> AppPage {
        let Some(d) = self.shop.details.clone() else {
            return AppPage::default();
        };
        let def = &d.definition;
        let mut p = AppPage {
            id: def.id.clone(),
            name: def.name.clone(),
            developer: def.developer.clone(),
            version: def.version.clone(),
            blurb: def.short_description.clone(),
            description: def.long_description.clone(),
            letter: letter(&def.name),
            color: tile_color(&def.accent_color, &def.name),
            rating: d.rating.map_or_else(String::new, |r| {
                format!("★ {:.1} ({} ratings)", r.average, r.count)
            }),
            price: price_text(d.price).to_owned(),
            cert: cert_text(d.certification).to_owned(),
            works_in: works_text(def.works_in),
            categories: def.categories.join(", "),
            rsc: def.rsc_permissions.clone(),
            device: def
                .device_permissions
                .iter()
                .map(|p| super::shop::permission_label(*p).to_owned())
                .collect(),
            targets: d
                .add_targets()
                .into_iter()
                .map(|t| match t {
                    app_store::AddTarget::Me => Nav::new("me", "Add for me"),
                    app_store::AddTarget::Team => Nav::new("team", "Add to a team"),
                    app_store::AddTarget::Chat => Nav::new("chat", "Add to a chat"),
                    app_store::AddTarget::Meeting => Nav::new("meeting", "Add to a meeting"),
                })
                .collect(),
            installed: self.is_installed(&def.id),
            pinned: self.is_pinned(&def.id),
            can_pin: def.has_personal_app && !def.is_core,
            opens_here: super::bar::native_key(def).is_some() || super::bar::app_view(def) != "app",
            privacy: def.privacy_url.clone().unwrap_or_default(),
            terms: def.terms_url.clone().unwrap_or_default(),
            shot_count: def.screenshots.len(),
            ..Default::default()
        };
        p.update = p.installed
            && self.installed.as_ref().is_some_and(|i| {
                i.entitlements
                    .iter()
                    .any(|e| e.id == def.id && e.has_update(Some(def)))
            });
        let (icon, shots) = (def.large_icon.clone(), def.screenshots.clone());
        p.icon = self.image(&icon);
        p.shots = shots.iter().map(|u| self.image(u)).collect();
        p
    }

    pub(super) fn store_data(&mut self) -> StoreData {
        let page = if self.shop.page.is_empty() {
            "home".to_owned()
        } else {
            self.shop.page.clone()
        };
        let mut d = StoreData {
            page: page.clone(),
            title: self.shop.title.clone(),
            status: self.shop.status.clone(),
            scope: match self.shop.scope {
                None => "all",
                Some(Scope::Personal) => "personal",
                Some(Scope::Chat) => "chat",
                Some(Scope::Team) => "team",
                Some(Scope::Meeting) => "meeting",
            }
            .into(),
            kind: match self.shop.agents {
                None => "all",
                Some(false) => "apps",
                Some(true) => "agents",
            }
            .into(),
            loading: self.shop.loading,
            featured: vec![
                Nav::new("popularApps", "Popular on Teams"),
                Nav::new("org", "Built for your org"),
                Nav::new("agents", "Agents"),
            ],
            categories: self
                .shop
                .home
                .iter()
                .flat_map(|h| h.categories.iter())
                .filter(|c| !matches!(c.key.as_str(), "all" | "popularApps") && c.total > 0)
                .map(|c| Nav {
                    key: c.key.clone(),
                    label: category_label(&c.key),
                    count: c.total.to_string(),
                })
                .collect(),
            ..Default::default()
        };
        match page.as_str() {
            "home" => {
                let ids = self.shop.category_ids("popularApps", STRIP);
                d.popular = self.id_cards(&ids);
                if d.popular.is_empty()
                    && self.shop.home.is_some()
                    && !self.shop.loading
                    && d.status.is_empty()
                {
                    d.status = "No apps match.".into();
                }
                let org: Vec<Card> = self
                    .shop
                    .org
                    .iter()
                    .take(STRIP)
                    .map(|a| Card {
                        id: a.id.clone(),
                        name: a.name.clone(),
                        developer: "Your organization".into(),
                        blurb: a.short_description.clone(),
                        letter: letter(&a.name),
                        color: tile_color("", &a.name),
                        ..Default::default()
                    })
                    .collect();
                d.org = org;
            }
            "results" => {
                let filter = self.shop.filter();
                let infos: Vec<Info> = self
                    .shop
                    .hits
                    .iter()
                    .filter(|h| filter.matches(h.works_in, h.is_agent))
                    .map(|h| Info::of_hit(h, self.shop.entry(&h.id)))
                    .collect();
                if infos.is_empty() && !self.shop.loading && self.shop.status.is_empty() {
                    d.status = "No apps match.".into();
                }
                d.cards = self.cards(infos);
            }
            "category" if self.shop.category == "org" => {
                d.cards = self
                    .shop
                    .org
                    .iter()
                    .map(|a| Card {
                        id: a.id.clone(),
                        name: a.name.clone(),
                        developer: "Your organization".into(),
                        blurb: a.short_description.clone(),
                        letter: letter(&a.name),
                        color: tile_color("", &a.name),
                        ..Default::default()
                    })
                    .collect();
            }
            "category" => {
                let ids = self
                    .shop
                    .category_ids(&self.shop.category.clone(), PAGE_SIZE);
                d.cards = self.id_cards(&ids);
                if d.cards.is_empty() && !self.shop.loading && self.shop.status.is_empty() {
                    d.status = "No apps match.".into();
                }
            }
            "app" => d.app = self.app_page(),
            "manage" => {
                let ids: Vec<String> = self.shop.mine.iter().map(|m| m.id.clone()).collect();
                let mut cards = self.id_cards(&ids);
                for c in &mut cards {
                    c.installed = true;
                }
                d.manage = cards;
                d.manage_note = "Remove and Update show the request Teams would get. kelma does \
                                 not change your apps."
                    .into();
            }
            "settings" => {
                d.policy = match self.policy {
                    Some(AppPolicy {
                        sideloading_allowed,
                        user_pinning_allowed,
                    }) => format!(
                        "Custom app upload: {}. Pinning apps to the bar: {}.",
                        if sideloading_allowed {
                            "allowed"
                        } else {
                            "not allowed"
                        },
                        if user_pinning_allowed {
                            "allowed"
                        } else {
                            "not allowed"
                        },
                    ),
                    None => String::new(),
                };
                d.perms = self.perm_rows();
            }
            _ => {}
        }
        if let Some(id) = self.shop.host.clone()
            && let Some(def) = self.def(&id)
        {
            d.host_name = def.name.clone();
            d.host_blurb = if def.short_description.is_empty() {
                def.long_description.clone()
            } else {
                def.short_description.clone()
            };
            d.host_link = host_link(def);
            d.host_tabs = def
                .personal_tabs()
                .map(|t| {
                    Nav::new(
                        &t.entity_id,
                        if t.name.is_empty() {
                            &t.entity_id
                        } else {
                            &t.name
                        },
                    )
                })
                .collect();
        }
        d
    }

    /// The Store settings rows: each device permission an installed app asks for, with the
    /// grant kelma keeps for it.
    fn perm_rows(&self) -> Vec<PermItem> {
        let Some(i) = &self.installed else {
            return Vec::new();
        };
        let mut rows: Vec<PermItem> = i
            .entitlements
            .iter()
            .filter(|e| e.state.is_installed())
            .filter_map(|e| i.definition(&e.id))
            .flat_map(|d| {
                d.device_permissions.iter().map(|p| PermItem {
                    app_id: d.id.clone(),
                    app_name: d.name.clone(),
                    perm: p.as_str().into(),
                    label: permission_label(*p).into(),
                    allowed: self.grants.allowed(&d.id, *p),
                })
            })
            .collect();
        rows.sort_by(|a, b| {
            (a.app_name.to_lowercase(), &a.perm).cmp(&(b.app_name.to_lowercase(), &b.perm))
        });
        rows
    }
}

/// What "Open in Teams" opens for an app: its first personal tab, else its Store page.
fn host_link(def: &AppDefinition) -> String {
    def.personal_tabs()
        .next()
        .map_or_else(|| def.share_link(), |t| def.open_link(&t.entity_id))
}

impl Engine {
    pub(super) fn shop_push(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().store_data();
        self.ui.apps_store(data);
        self.apps_fetch_images();
    }

    fn shop_edit(self: &Arc<Self>, f: impl FnOnce(&mut Shop)) {
        f(&mut self.apps.lock().unwrap().shop);
        self.shop_push();
    }

    /// Fetches the definitions of the apps that are not known yet.
    async fn shop_ensure_defs(self: &Arc<Self>, ids: &[String]) {
        let missing: Vec<String> = {
            let a = self.apps.lock().unwrap();
            ids.iter().filter(|id| !a.has_def(id)).cloned().collect()
        };
        for chunk in missing.chunks(100) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            match self.src_defs(&refs).await {
                Ok(defs) => {
                    let mut a = self.apps.lock().unwrap();
                    for d in defs {
                        a.defs.insert(d.id.clone(), d);
                    }
                }
                Err(err) => tracing::warn!(%err, "app definitions failed"),
            }
        }
    }

    fn shop_fail(self: &Arc<Self>, what: &str, err: &anyhow::Error) {
        tracing::warn!(%err, "{what} failed");
        self.shop_edit(|s| {
            s.loading = false;
            s.status = format!("{what} failed: {err}");
        });
    }

    /// The Store view appeared (the rail, the dev view switch): load it unless it is loaded.
    pub(super) async fn shop_enter(self: &Arc<Self>) -> anyhow::Result<()> {
        let busy = {
            let s = &self.apps.lock().unwrap().shop;
            s.home.is_some() || s.loading
        };
        if busy {
            self.shop_push();
            return Ok(());
        }
        self.shop_home().await
    }

    pub(super) async fn shop_home(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("store".into());
        self.shop_edit(|s| {
            s.page = "home".into();
            s.loading = true;
            s.status.clear();
        });
        self.apps_ensure_installed().await;
        if self.apps.lock().unwrap().shop.home.is_none() {
            match self.src_home().await {
                Ok(h) => self.apps.lock().unwrap().shop.home = Some(h),
                Err(err) => {
                    self.shop_fail("Loading the Store", &err);
                    return Ok(());
                }
            }
            // "Built for your org" is optional: Graph may not grant the catalog.
            let org = self.src_org().await.unwrap_or_default();
            self.apps.lock().unwrap().shop.org = org;
        }
        let ids = self
            .apps
            .lock()
            .unwrap()
            .shop
            .category_ids("popularApps", STRIP);
        self.shop_ensure_defs(&ids).await;
        self.shop_edit(|s| s.loading = false);
        Ok(())
    }

    pub(super) async fn shop_search(self: &Arc<Self>, query: String) -> anyhow::Result<()> {
        let q = query.trim().to_owned();
        if q.is_empty() {
            return self.shop_home().await;
        }
        self.ui.open_view("store".into());
        self.shop_edit(|s| {
            s.page = "results".into();
            s.title = format!("Results for “{q}”");
            s.loading = true;
            s.status.clear();
            s.hits.clear();
        });
        self.apps_ensure_installed().await;
        match self.src_search(&q).await {
            Ok(hits) => self.shop_edit(|s| {
                s.hits = hits;
                s.loading = false;
            }),
            Err(err) => self.shop_fail("Search", &err),
        }
        Ok(())
    }

    pub(super) async fn shop_category(self: &Arc<Self>, key: &str) -> anyhow::Result<()> {
        // The Agents entry of Featured is every app, filtered to agents.
        let key = if key == "agents" {
            self.shop_edit(|s| s.agents = Some(true));
            "all"
        } else {
            key
        };
        self.ui.open_view("store".into());
        let title = if key == "org" {
            "Built for your org".to_owned()
        } else if key == "popularApps" {
            "Popular on Teams".to_owned()
        } else {
            category_label(key)
        };
        self.shop_edit(|s| {
            s.page = "category".into();
            s.category = key.to_owned();
            s.title = title;
            s.loading = true;
            s.status.clear();
        });
        self.apps_ensure_installed().await;
        if self.apps.lock().unwrap().shop.home.is_none() {
            match self.src_home().await {
                Ok(h) => self.apps.lock().unwrap().shop.home = Some(h),
                Err(err) => {
                    self.shop_fail("Loading the Store", &err);
                    return Ok(());
                }
            }
        }
        let ids = self.apps.lock().unwrap().shop.category_ids(key, PAGE_SIZE);
        self.shop_ensure_defs(&ids).await;
        self.shop_edit(|s| s.loading = false);
        Ok(())
    }

    pub(super) async fn shop_scope(self: &Arc<Self>, s: &str) -> anyhow::Result<()> {
        let scope = parse_scope(s);
        self.shop_edit(|shop| shop.scope = scope);
        Ok(())
    }

    pub(super) async fn shop_kind(self: &Arc<Self>, k: &str) -> anyhow::Result<()> {
        let agents = match k {
            "apps" => Some(false),
            "agents" => Some(true),
            _ => None,
        };
        self.shop_edit(|shop| shop.agents = agents);
        Ok(())
    }

    /// Opens an app's page.
    pub(super) async fn shop_app(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        self.ui.open_view("store".into());
        self.apps_ensure_installed().await;
        self.shop_ensure_defs(&[id.to_owned()]).await;
        let found = {
            let mut a = self.apps.lock().unwrap();
            match a.def(id).cloned() {
                Some(def) => {
                    let hit = a.shop.hits.iter().find(|h| h.id == id).cloned();
                    let details = AppDetails::new(def, a.shop.entry(id), hit.as_ref());
                    a.shop.details = Some(details);
                    a.shop.page = "app".into();
                    a.shop.status.clear();
                    true
                }
                None => false,
            }
        };
        if !found {
            self.shop_edit(|s| {
                s.status = "This app is not in the Store listing.".into();
            });
            return Ok(());
        }
        self.shop_push();
        Ok(())
    }

    /// The definition and its name, for the install dialogs.
    fn shop_def(&self, id: &str) -> Option<AppDefinition> {
        self.apps.lock().unwrap().def(id).cloned()
    }

    /// Shows the request an install, uninstall or update would send. It is never sent.
    fn shop_show_request(
        self: &Arc<Self>,
        title: String,
        req: &app_store::Request,
        note: &str,
        link: String,
    ) {
        tracing::info!(request = %req.dry_run().replace('\n', " "), "app change not sent");
        self.ui.apps_install(
            title,
            format!(
                "{}\n\n{note}\nNothing was sent. kelma does not change your apps; use Open in \
                 Teams to do it there.",
                req.dry_run()
            ),
            link,
        );
    }

    pub(super) async fn shop_add(self: &Arc<Self>, target: &str) -> anyhow::Result<()> {
        let Some(def) = self
            .apps
            .lock()
            .unwrap()
            .shop
            .details
            .as_ref()
            .map(|d| d.definition.clone())
        else {
            return Ok(());
        };
        let (t, label, note) = match target {
            "team" => (
                Target::Team("<team id>".into()),
                "to a team",
                "Pick the team in Teams.",
            ),
            "chat" => (
                Target::Chat("<chat id>".into()),
                "to a chat",
                "Pick the chat in Teams.",
            ),
            "meeting" => (
                Target::Meeting("<meeting chat id>".into()),
                "to a meeting",
                "Pick the meeting in Teams.",
            ),
            _ => (Target::Me, "for you", ""),
        };
        let base = self.apps_base().await;
        let req = install_request(&base, &t, &def.raw);
        self.shop_show_request(
            format!("Add {} {label}", def.name),
            &req,
            note,
            def.share_link(),
        );
        Ok(())
    }

    pub(super) async fn shop_remove(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let Some(def) = self.shop_def(id) else {
            return Ok(());
        };
        let base = self.apps_base().await;
        let req = uninstall_request(&base, &Target::Me, id);
        self.shop_show_request(format!("Remove {}", def.name), &req, "", def.share_link());
        Ok(())
    }

    pub(super) async fn shop_update(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let Some(def) = self.shop_def(id) else {
            return Ok(());
        };
        let base = self.apps_base().await;
        let req = update_request(&base, &Target::Me, id, &def.version);
        self.shop_show_request(format!("Update {}", def.name), &req, "", def.share_link());
        Ok(())
    }

    pub(super) fn shop_share(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let Some(def) = self.shop_def(id) else {
            return Ok(());
        };
        let link = def.share_link();
        if self.apps_offline() {
            self.apps
                .lock()
                .unwrap()
                .opened
                .push(format!("copied:{link}"));
        } else {
            arboard::Clipboard::new()?.set_text(link)?;
        }
        self.ui.banner(format!("Link to {} copied.", def.name));
        Ok(())
    }

    /// `privacy`, `terms`, `devices`, `web`, `host`, `tab:<entity id>` or `url:<https link>`.
    pub(super) fn shop_link(self: &Arc<Self>, what: &str) -> anyhow::Result<()> {
        // A W app's tab opens in the web host; the rest of the links go to the browser.
        if let Some(tab) = what.strip_prefix("tab:")
            && self.apps_open_in_host(tab)
        {
            return Ok(());
        }
        let url = {
            let a = self.apps.lock().unwrap();
            let details = a.shop.details.as_ref().map(|d| &d.definition);
            let host = a.shop.host.as_ref().and_then(|h| a.def(h));
            match what {
                "devices" => Some(DEVICES_URL.to_owned()),
                "privacy" => details.and_then(|d| d.privacy_url.clone()),
                "terms" => details.and_then(|d| d.terms_url.clone()),
                "web" => details.map(AppDefinition::share_link),
                "host" => host.map(host_link),
                other if other.starts_with("url:") => Some(other[4..].to_owned()),
                tab => tab
                    .strip_prefix("tab:")
                    .and_then(|e| host.map(|d| d.open_link(e))),
            }
        };
        if let Some(url) = url {
            self.apps_open_link(&url);
        }
        Ok(())
    }

    pub(super) async fn shop_manage(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("store".into());
        self.shop_edit(|s| {
            s.page = "manage".into();
            s.title = "Manage your apps".into();
            s.loading = true;
            s.status.clear();
        });
        self.apps_ensure_installed().await;
        match self.src_mine().await {
            Ok(mine) => {
                let ids: Vec<String> = mine.iter().map(|m| m.id.clone()).collect();
                self.shop_ensure_defs(&ids).await;
                self.shop_edit(|s| {
                    s.mine = mine;
                    s.loading = false;
                });
            }
            Err(err) => self.shop_fail("Loading your apps", &err),
        }
        Ok(())
    }

    pub(super) async fn shop_settings(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("store".into());
        self.apps_ensure_installed().await;
        let policy = self.src_policy().await.ok();
        {
            let mut a = self.apps.lock().unwrap();
            if !a.grants_loaded {
                a.grants_loaded = true;
                if !self.apps_offline()
                    && let Some(g) = load_grants()
                {
                    a.grants = g;
                }
            }
            a.policy = policy.or(a.policy);
        }
        self.shop_edit(|s| {
            s.page = "settings".into();
            s.title = "Store settings".into();
            s.loading = false;
            s.status.clear();
        });
        Ok(())
    }

    pub(super) fn shop_grant(
        self: &Arc<Self>,
        app: &str,
        perm: &str,
        on: bool,
    ) -> anyhow::Result<()> {
        let Some(p) = DevicePermission::parse(perm) else {
            return Ok(());
        };
        let json = {
            let mut a = self.apps.lock().unwrap();
            a.grants.set(app, p, on);
            a.grants.to_json()
        };
        if !self.apps_offline() {
            save_grants(&json);
        }
        self.shop_push();
        Ok(())
    }
}

fn grants_path() -> Option<std::path::PathBuf> {
    Some(
        session::Paths::new()
            .ok()?
            .config_dir
            .join("app-grants.json"),
    )
}

/// The text of `app-grants.json`, if there is one (the web host reads the grants from it).
pub(super) fn read_grants_file() -> Option<String> {
    std::fs::read_to_string(grants_path()?).ok()
}

fn load_grants() -> Option<app_store::Grants> {
    let text = std::fs::read_to_string(grants_path()?).ok()?;
    Some(app_store::Grants::from_json(
        &serde_json::from_str(&text).ok()?,
    ))
}

fn save_grants(json: &serde_json::Value) {
    let Some(path) = grants_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json.to_string()).is_ok() {
        let _ = std::fs::rename(tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_keys_become_labels() {
        assert_eq!(category_label("popularApps"), "Popular apps");
        assert_eq!(category_label("productivity"), "Productivity");
        assert_eq!(category_label("humanResources"), "Human resources");
    }
}
