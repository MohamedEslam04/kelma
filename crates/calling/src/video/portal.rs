//! Tells the desktop portal which app is asking, before the first portal call.
//!
//! A kelma started from a terminal is not a registered app, so the portal names it after its
//! parent process ("Konsole" in KDE's ScreenCast and RemoteDesktop dialogs). The host portal
//! registry (`org.freedesktop.host.portal.Registry.Register`, xdg-desktop-portal 1.19 and newer)
//! lets a host app say which desktop file it is; `ashpd::register_host_app` calls it on the
//! connection every later portal call shares. A Flatpak or Snap is named by its sandbox, so
//! ashpd skips it. A portal without the registry (older) refuses: that is logged and the call goes
//! on, with the portal's own naming.
//!
//! The app sets its id once at start-up ([`set_host_app_id`]); every portal entry point awaits
//! [`ensure_registered`] first, so the registration happens once and before any other call.

use std::sync::OnceLock;
use tokio::sync::OnceCell;

static APP_ID: OnceLock<String> = OnceLock::new();
static REGISTERED: OnceCell<()> = OnceCell::const_new();

/// Names the app for the portal (a reverse-DNS desktop file id). The first call wins.
pub fn set_host_app_id(id: &str) {
    let _ = APP_ID.set(id.to_owned());
}

/// Registers the app id with the portal, once. Never fails: a refusal only means the portal
/// keeps naming the app its own way.
pub(crate) async fn ensure_registered() {
    REGISTERED
        .get_or_init(|| async {
            if let Some(id) = APP_ID.get() {
                register(id).await;
            }
        })
        .await;
}

async fn register(id: &str) {
    let app_id = match ashpd::AppID::try_from(id) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(%e, id, "not a valid portal app id; the portal names kelma itself");
            return;
        }
    };
    match ashpd::register_host_app(app_id).await {
        Ok(()) => tracing::debug!(id, "registered with the portal"),
        Err(e) => tracing::info!(%e, id, "portal app registration refused (older portal?)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The portal refuses an id that is not reverse-DNS (`kelma` alone has one segment).
    #[test]
    fn the_portal_accepts_reverse_dns_ids_only() {
        assert!(ashpd::AppID::try_from("io.github.example.app").is_ok());
        assert!(ashpd::AppID::try_from("kelma").is_err());
    }

    #[tokio::test]
    async fn without_an_id_nothing_is_registered_and_nothing_fails() {
        // No id is set in this test binary: no D-Bus call is made, and asking again is fine.
        ensure_registered().await;
        ensure_registered().await;
        assert!(APP_ID.get().is_none());
    }
}
