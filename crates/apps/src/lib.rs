//! Teams apps for kelma: the app bar, the "View more apps" flyout, the Store (home, search,
//! app page, Manage your apps), install requests, the Store settings, and the bots and
//! messaging extensions installed in a chat or team.
//!
//! Everything reads the Teams app service on the middle tier (`beta/users/apps/…`) with the
//! client's Teams credentials; the org catalog comes from Graph (`AppCatalog.Read.All`). Graph's
//! `TeamsAppInstallation.*ForUser` is not granted, so the user's apps come from the app service
//! too. Install, uninstall, update and pin requests are only built (dry run), never sent.
//! Parsers and request builders are pure functions with their own tests.

pub mod api;
pub mod bar;
pub mod bots;
pub mod install;
pub mod model;
pub mod settings;
pub mod store;

pub use bar::{
    Entitlement, EntitlementState, InstalledApps, PinChange, installed_apps, parse_installed,
    pin_request,
};
pub use install::{Request, Target, install_request, uninstall_request, update_request};
pub use model::{AppDefinition, DevicePermission, StaticTab, WorksIn, parse_definitions};
pub use settings::{AppPolicy, Grants, app_policy, parse_app_policy};
pub use store::{
    AddTarget, AppDetails, CatalogApp, Category, Certification, Distribution, Filter, Price,
    Rating, Scope, SearchHit, StoreEntry, StoreHome, YourApp, catalog_apps, definitions, org_apps,
    parse_search, parse_store_home, parse_your_apps, search, store_home, store_home_body,
    your_apps,
};
