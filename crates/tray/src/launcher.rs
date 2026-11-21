//! Unity LauncherEntry badge: the `com.canonical.Unity.LauncherEntry.Update` signal, which the
//! KDE task manager (and Dash to Dock, Plank, ...) show as a count on the launcher.

#[cfg(target_os = "linux")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use zbus::zvariant::Value;

/// Desktop file id of the app, as a launcher URI. The task manager matches it to the entry.
/// The launcher URI of [`crate::APP_ID`]'s desktop file.
pub const APP_URI: &str = "application://io.github.mohamedeslam04.kelma.desktop";
pub const OBJECT_PATH: &str = "/com/canonical/unity/launcherentry/kelma";
pub const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
pub const SIGNAL: &str = "Update";

/// The body of one `Update(s app_uri, a{sv} properties)` signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherUpdate {
    pub app_uri: String,
    pub count: i64,
    pub count_visible: bool,
}

/// Build the update for `count` (0 hides the badge).
pub fn launcher_update(app_uri: &str, count: u32) -> LauncherUpdate {
    LauncherUpdate {
        app_uri: app_uri.to_string(),
        count: i64::from(count),
        count_visible: count > 0,
    }
}

#[cfg(target_os = "linux")]
impl LauncherUpdate {
    /// The signal arguments: `(s, a{sv})` with `count` (x) and `count-visible` (b).
    pub fn body(&self) -> (String, HashMap<&'static str, Value<'static>>) {
        let props = HashMap::from([
            ("count", Value::I64(self.count)),
            ("count-visible", Value::Bool(self.count_visible)),
        ]);
        (self.app_uri.clone(), props)
    }

    /// Broadcast the update on `conn` (no destination, as the spec asks).
    pub async fn emit(&self, conn: &zbus::Connection) -> zbus::Result<()> {
        conn.emit_signal(None::<&str>, OBJECT_PATH, INTERFACE, SIGNAL, &self.body())
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_hides_the_badge() {
        let u = launcher_update(APP_URI, 0);
        assert_eq!((u.count, u.count_visible), (0, false));
    }

    #[test]
    fn nonzero_shows_the_count() {
        let u = launcher_update(APP_URI, 12);
        assert_eq!((u.count, u.count_visible), (12, true));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn body_has_uri_and_typed_properties() {
        let (uri, props) = launcher_update(APP_URI, 7).body();
        assert_eq!(uri, "application://io.github.mohamedeslam04.kelma.desktop");
        assert_eq!(props.len(), 2);
        assert_eq!(props["count"], Value::I64(7));
        assert_eq!(props["count-visible"], Value::Bool(true));
        // The wire type is (sa{sv}); count is a 64-bit int per the LauncherEntry spec.
        assert_eq!(props["count"].value_signature().to_string(), "x");
    }
}
