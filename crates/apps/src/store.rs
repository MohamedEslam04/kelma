//! The Store: home (categories with ranked app ids), search, app definitions for the app page,
//! "Manage your apps", and the org catalog from Graph.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::graph::{graph_url, str_of};
use teams_proto::{Client, Result};

use crate::api;
use crate::model::{AppDefinition, WorksIn, parse_definitions};

/// A Store rating.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rating {
    pub average: f64,
    pub count: u32,
}

/// An app in a Store category. Only ids and ranking: the names and icons come from
/// [`definitions`] (the web client does the same).
#[derive(Debug, Clone, PartialEq)]
pub struct StoreEntry {
    pub id: String,
    pub version: String,
    pub ranking: Option<u32>,
    pub rating: Option<Rating>,
    /// `StaticTabs`, `Bots`, `MessageExtensions`, `CopilotExtensions`, …
    pub kinds: Vec<String>,
    /// Blocked by the admin (shown with "Request approval").
    pub is_blocked: bool,
}

/// One Store category: `all`, `popularApps`, `productivity`, `personalApps`, `bots`, …
#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    pub key: String,
    pub total: u32,
    pub blocked: u32,
    pub apps: Vec<StoreEntry>,
}

/// The Store home.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StoreHome {
    pub categories: Vec<Category>,
}

impl StoreHome {
    pub fn category(&self, key: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.key == key)
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn u32_of(v: &Value) -> Option<u32> {
    v.as_u64().map(|n| n as u32)
}

fn entry(v: &Value) -> Option<StoreEntry> {
    Some(StoreEntry {
        id: str_of(v, "id")?,
        version: str_of(v, "version").unwrap_or_default(),
        ranking: u32_of(&v["ranking"]),
        rating: v["rating"]["average"].as_f64().map(|average| Rating {
            average,
            count: u32_of(&v["rating"]["count"]).unwrap_or(0),
        }),
        kinds: strings(&v["metaCapabilities"]),
        is_blocked: v["isBlocked"].as_bool().unwrap_or(false),
    })
}

/// Parses the Store home answer (`appCategoriesDictionary`), categories by key.
pub fn parse_store_home(v: &Value) -> StoreHome {
    let mut categories: Vec<Category> = v["appCategoriesDictionary"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, c)| Category {
            key: key.clone(),
            total: u32_of(&c["totalAppsCount"]).unwrap_or(0),
            blocked: u32_of(&c["blockedAppsCount"]).unwrap_or(0),
            apps: c["apps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(entry)
                .collect(),
        })
        .collect();
    categories.sort_by(|a, b| a.key.cmp(&b.key));
    StoreHome { categories }
}

/// The Store home body (`GetAppsStoreResponseModel`). `categories` empty means all categories.
pub fn store_home_body(categories: &[&str]) -> Value {
    json!({
        "appCategories": categories,
        "appsStoreOptions": {
            "includeIndustries": true,
            "includeSuggestedApps": true,
            "includeTotalAppsCount": true,
        },
    })
}

/// The Store home: every category with its first 100 ranked app ids and totals.
pub async fn store_home(client: &Client) -> Result<StoreHome> {
    let v = api::read(
        client,
        "store",
        Method::POST,
        "beta/users/apps/store",
        Some(&store_home_body(&[])),
    )
    .await?;
    Ok(parse_store_home(&v))
}

/// The price shown on the app page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Price {
    Free,
    Paid,
    Unknown,
}

/// The publisher's compliance level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Certification {
    None,
    PublisherAttested,
    Certified,
}

/// A Store search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub id: String,
    /// The Microsoft 365 title id of agents and unified apps (`T_…`).
    pub title_id: Option<String>,
    pub name: String,
    pub developer: String,
    pub version: String,
    pub short_description: String,
    pub accent_color: String,
    pub icon: String,
    pub categories: Vec<String>,
    pub screenshots: Vec<String>,
    pub kinds: Vec<String>,
    pub works_in: WorksIn,
    pub is_agent: bool,
    pub price: Price,
    pub certification: Certification,
    pub score: f64,
}

/// `features` of a hit name scopes (`Personal`, `NoContextTeamTab`, `GroupChatContextBot`,
/// `Meeting`, …), the price (`Free`, `Paid`) and the certification.
fn features(f: &[String]) -> (WorksIn, bool, Price, Certification) {
    let has = |s: &str| f.iter().any(|x| x == s);
    let any = |p: &dyn Fn(&str) -> bool| f.iter().any(|x| p(x));
    let works_in = WorksIn {
        personal: has("Personal") || any(&|x| x.starts_with("PersonalContext")),
        team: any(&|x| {
            x == "NoContextTeamTab" || x == "TeamContextBot" || x.starts_with("ChannelContext")
        }),
        chat: any(&|x| x == "NoContextChatTab" || x == "GroupChatContextBot"),
        meeting: has("Meeting") || any(&|x| x.starts_with("Meeting")),
    };
    let is_agent = any(&|x| {
        matches!(
            x,
            "CopilotExtension" | "CopilotPlugin" | "Gpt" | "CustomCopilot"
        )
    });
    let price = if has("Paid") {
        Price::Paid
    } else if has("Free") {
        Price::Free
    } else {
        Price::Unknown
    };
    let certification = if has("Certified") {
        Certification::Certified
    } else if has("PublisherAttested") {
        Certification::PublisherAttested
    } else {
        Certification::None
    };
    (works_in, is_agent, price, certification)
}

/// Parses a search answer (`results`), best first as served.
pub fn parse_search(v: &Value) -> Vec<SearchHit> {
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            let (works_in, is_agent, price, certification) = features(&strings(&h["features"]));
            Some(SearchHit {
                id: str_of(h, "appId")?,
                title_id: str_of(h, "titleId"),
                name: str_of(h, "name").unwrap_or_default(),
                developer: str_of(h, "developerName").unwrap_or_default(),
                version: str_of(h, "version").unwrap_or_default(),
                short_description: str_of(h, "shortDescription").unwrap_or_default(),
                accent_color: str_of(h, "accentColor").unwrap_or_default(),
                icon: str_of(h, "smallImageUrl").unwrap_or_default(),
                categories: strings(&h["appCategories"]),
                screenshots: strings(&h["screenshotUrls"]),
                kinds: strings(&h["capabilities"]),
                works_in,
                is_agent,
                price,
                certification,
                score: h["searchScore"].as_f64().unwrap_or(0.0),
            })
        })
        .collect()
}

/// Searches the Store ("Search agents and apps").
pub async fn search(client: &Client, query: &str) -> Result<Vec<SearchHit>> {
    let body = json!({"searchQuery": query, "includeOpenAiPlugins": true});
    let v = api::read(
        client,
        "app search",
        Method::POST,
        "beta/users/apps/search",
        Some(&body),
    )
    .await?;
    Ok(parse_search(&v))
}

/// Full definitions of the given apps (`batchedDefinitions`, body: the app ids), for the app
/// page and for Store rows. Unknown ids are left out of the answer.
pub async fn definitions(client: &Client, ids: &[&str]) -> Result<Vec<AppDefinition>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let v = api::read(
        client,
        "app definitions",
        Method::POST,
        "beta/users/apps/batchedDefinitions",
        Some(&json!(ids)),
    )
    .await?;
    Ok(parse_definitions(&v))
}

/// An app in "Manage your apps".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YourApp {
    pub id: String,
    pub version: String,
    pub categories: Vec<String>,
    pub kinds: Vec<String>,
}

/// Parses `appIdToStoreEntitlementsDictionary`, by app id.
pub fn parse_your_apps(v: &Value) -> Vec<YourApp> {
    let mut apps: Vec<YourApp> = v["appIdToStoreEntitlementsDictionary"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, a)| YourApp {
            id: str_of(a, "id").unwrap_or_else(|| id.clone()),
            version: str_of(a, "version").unwrap_or_default(),
            categories: strings(&a["categories"]),
            kinds: strings(&a["metaCapabilities"]),
        })
        .collect();
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    apps
}

/// "Manage your apps": the Store apps the user added.
pub async fn your_apps(client: &Client) -> Result<Vec<YourApp>> {
    let v = api::read(
        client,
        "your apps",
        Method::GET,
        "beta/users/apps/store/entitlements",
        None,
    )
    .await?;
    Ok(parse_your_apps(&v))
}

/// A works-in filter of the Store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Personal,
    Team,
    Chat,
    Meeting,
}

/// The Store filters: works in, and agents or apps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Filter {
    pub scope: Option<Scope>,
    /// `Some(true)` agents only, `Some(false)` apps only.
    pub agents: Option<bool>,
}

impl Filter {
    pub fn matches(&self, works_in: WorksIn, is_agent: bool) -> bool {
        let scope_ok = match self.scope {
            None => true,
            Some(Scope::Personal) => works_in.personal,
            Some(Scope::Team) => works_in.team,
            Some(Scope::Chat) => works_in.chat,
            Some(Scope::Meeting) => works_in.meeting,
        };
        scope_ok && self.agents.is_none_or(|a| a == is_agent)
    }
}

/// Where the app page's Add ▾ can add an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddTarget {
    Me,
    Team,
    Chat,
    Meeting,
}

/// The app page: the definition plus what only the Store knows (rating from the Store home,
/// price and certification from a search hit). Install counts are not served to the client.
#[derive(Debug, Clone, PartialEq)]
pub struct AppDetails {
    pub definition: AppDefinition,
    pub rating: Option<Rating>,
    pub price: Price,
    pub certification: Certification,
}

impl AppDetails {
    pub fn new(
        definition: AppDefinition,
        entry: Option<&StoreEntry>,
        hit: Option<&SearchHit>,
    ) -> Self {
        AppDetails {
            definition,
            rating: entry.and_then(|e| e.rating),
            price: hit.map_or(Price::Unknown, |h| h.price),
            certification: hit.map_or(Certification::None, |h| h.certification),
        }
    }

    /// The Add ▾ menu, in Teams order.
    pub fn add_targets(&self) -> Vec<AddTarget> {
        let w = self.definition.works_in;
        [
            (w.personal, AddTarget::Me),
            (w.team, AddTarget::Team),
            (w.chat, AddTarget::Chat),
            (w.meeting, AddTarget::Meeting),
        ]
        .into_iter()
        .filter_map(|(ok, t)| ok.then_some(t))
        .collect()
    }
}

/// How an app reached the Graph catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Distribution {
    Store,
    /// "Built for your org".
    Organization,
    Sideloaded,
    Other(String),
}

/// An app of the Graph catalog (`appCatalogs/teamsApps`, `AppCatalog.Read.All`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogApp {
    pub id: String,
    pub external_id: Option<String>,
    pub name: String,
    pub distribution: Distribution,
    pub version: String,
    pub short_description: String,
    pub description: String,
    pub has_bot: bool,
    pub rsc_permissions: Vec<String>,
}

/// Parses Graph `teamsApp` items (with `appDefinitions` expanded), skipping items without an id.
pub fn catalog_apps(items: &[Value]) -> Vec<CatalogApp> {
    items
        .iter()
        .filter_map(|a| {
            let def = &a["appDefinitions"][0];
            Some(CatalogApp {
                id: str_of(a, "id")?,
                external_id: str_of(a, "externalId"),
                name: str_of(a, "displayName").unwrap_or_default(),
                distribution: match a["distributionMethod"].as_str().unwrap_or_default() {
                    "store" => Distribution::Store,
                    "organization" => Distribution::Organization,
                    "sideloaded" => Distribution::Sideloaded,
                    other => Distribution::Other(other.to_owned()),
                },
                version: str_of(def, "version").unwrap_or_default(),
                short_description: str_of(def, "shortDescription").unwrap_or_default(),
                description: str_of(def, "description").unwrap_or_default(),
                has_bot: def["bot"].is_object(),
                rsc_permissions:
                    def["authorization"]["requiredPermissionSet"]["resourceSpecificPermissions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| str_of(p, "permissionValue"))
                        .collect(),
            })
        })
        .collect()
}

/// "Built for your org": the tenant's own apps from the Graph catalog.
pub async fn org_apps(client: &Client) -> Result<Vec<CatalogApp>> {
    let url = graph_url(
        &["appCatalogs", "teamsApps"],
        "$filter=distributionMethod eq 'organization'&$expand=appDefinitions",
    );
    let items = client.graph_values("org apps", &url, 5).await?;
    Ok(catalog_apps(&items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_home_body_names_the_categories() {
        let b = store_home_body(&["popularApps"]);
        assert_eq!(b["appCategories"], json!(["popularApps"]));
        assert_eq!(b["appsStoreOptions"]["includeTotalAppsCount"], true);
    }

    #[test]
    fn features_without_markers_are_unknown() {
        let (w, agent, price, cert) = features(&[]);
        assert_eq!(w, WorksIn::default());
        assert!(!agent);
        assert_eq!(price, Price::Unknown);
        assert_eq!(cert, Certification::None);
    }
}
