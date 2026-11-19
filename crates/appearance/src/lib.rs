//! kelma appearance preferences.
//!
//! - [`AppearancePrefs`]: theme, accent, density and font scale, kept in `appearance.json` in the
//!   app config dir (atomic writes; a missing or corrupt file gives the defaults).
//! - [`portal`]: reads and watches the desktop's color scheme and accent over the XDG portal.
//! - [`resolve`]: pure functions that combine the two into what the UI should draw.

pub mod portal;
pub mod prefs;
pub mod resolve;
mod store;

pub use portal::{PortalChange, PortalError, PortalScheme, PortalSettings, PortalStream};
pub use prefs::{AppearancePrefs, Density, FONT_SCALE_MAX, FONT_SCALE_MIN, Rgb, Theme};
pub use resolve::{
    Resolved, ResolvedTheme, resolve, resolve_accent, resolve_font_scale, resolve_theme,
};
pub use store::{Error, config_dir};
