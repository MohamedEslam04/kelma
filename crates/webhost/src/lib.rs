//! kelma's web host: opens a web app in a Chromium-family `--app` window with its own profile,
//! drives it over the Chrome DevTools Protocol and answers TeamsJS calls from Rust.
//!
//! See `docs/adr/0003-web-host.md` for why this is a Chromium window and not WebKitGTK.

pub mod browser;
pub mod cdp;
pub mod shim;
mod window;

use std::path::PathBuf;

/// Web host errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "no Chromium-family browser found (looked for {}); set KELMA_BROWSER or the browser path",
        browser::CANDIDATES.join(", ")
    )]
    NoBrowser,
    #[error("the web host profile {} is already open in another browser", .0.display())]
    ProfileInUse(PathBuf),
    #[error("the browser did not start: {0}")]
    Startup(String),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("DevTools connection: {0}")]
    Ws(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("CDP {method} failed: {message}")]
    Cdp { method: String, message: String },
    #[error("page script failed: {0}")]
    Script(String),
    #[error("the web host window is closed")]
    Closed,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

pub use shim::{AppEvent, HostContext, ShimConfig, TokenRequest};
pub use window::{
    BoxFuture, BridgeCall, BridgeHandler, TokenCallback, WebOptions, WebWindow,
    default_profile_dir, open,
};
