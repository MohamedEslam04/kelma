//! The XDG desktop portal (`org.freedesktop.portal.Settings`): read the desktop's color scheme
//! and accent color, and watch them change. Parsing is pure; only [`read`] and [`watch`] talk to
//! D-Bus. zbus runs on its own async-io reactor, so they work from any executor.
//!
//! Without a session bus or a portal, [`read`] and [`watch`] return `Ok(None)`. They never panic.

use crate::prefs::Rgb;
use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use zbus::zvariant::{OwnedValue, Value};

pub const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SETTINGS_IFACE: &str = "org.freedesktop.portal.Settings";

pub const NS_APPEARANCE: &str = "org.freedesktop.appearance";
pub const KEY_COLOR_SCHEME: &str = "color-scheme";
pub const KEY_ACCENT_COLOR: &str = "accent-color";
/// `contrast` (u32): 0 normal, 1 high.
pub const KEY_CONTRAST: &str = "contrast";
/// GNOME's animation switch, readable through the portal on GNOME desktops only.
pub const NS_GNOME_INTERFACE: &str = "org.gnome.desktop.interface";
pub const KEY_ENABLE_ANIMATIONS: &str = "enable-animations";

/// The portal's `color-scheme`: 0 no preference, 1 prefer dark, 2 prefer light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortalScheme {
    #[default]
    NoPreference,
    Dark,
    Light,
}

/// What the portal currently says. Missing keys keep their defaults.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PortalSettings {
    pub color_scheme: PortalScheme,
    /// The desktop accent color, `None` when the portal has none.
    pub accent: Option<Rgb>,
    /// The desktop asks for high contrast (`contrast` is 1).
    pub high_contrast: bool,
    /// `enable-animations`; `None` when the desktop does not expose it (KDE, for one).
    pub animations_enabled: Option<bool>,
}

/// One live change from the `SettingChanged` signal.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PortalChange {
    ColorScheme(PortalScheme),
    Accent(Option<Rgb>),
    HighContrast(bool),
    AnimationsEnabled(bool),
}

impl PortalSettings {
    /// Folds a live change into the current state.
    pub fn apply(&mut self, change: PortalChange) {
        match change {
            PortalChange::ColorScheme(s) => self.color_scheme = s,
            PortalChange::Accent(a) => self.accent = a,
            PortalChange::HighContrast(h) => self.high_contrast = h,
            PortalChange::AnimationsEnabled(e) => self.animations_enabled = Some(e),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("desktop portal: {0}")]
pub struct PortalError(#[from] pub zbus::Error);

/// Live changes. Ends if the bus connection drops.
pub type PortalStream = Pin<Box<dyn Stream<Item = PortalChange> + Send>>;

// ---- pure parsing -------------------------------------------------------------------------

/// Strips the extra variant layers the legacy `Read` method adds.
pub(crate) fn unwrap_variant<'a, 'b>(mut v: &'b Value<'a>) -> &'b Value<'a> {
    while let Value::Value(inner) = v {
        v = inner;
    }
    v
}

pub(crate) fn parse_u32(v: &Value<'_>) -> Option<u32> {
    match unwrap_variant(v) {
        Value::U32(n) => Some(*n),
        _ => None,
    }
}

pub(crate) fn parse_bool(v: &Value<'_>) -> Option<bool> {
    match unwrap_variant(v) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// `contrast` (u32): 1 is high contrast, any other number is normal. Another type is `None`.
pub fn parse_contrast(v: &Value<'_>) -> Option<bool> {
    parse_u32(v).map(|n| n == 1)
}

/// `color-scheme` (u32). Unknown numbers mean no preference, as the portal spec says;
/// a value of another type is `None`.
pub fn parse_color_scheme(v: &Value<'_>) -> Option<PortalScheme> {
    parse_u32(v).map(|n| match n {
        1 => PortalScheme::Dark,
        2 => PortalScheme::Light,
        _ => PortalScheme::NoPreference,
    })
}

/// `accent-color` (`(ddd)`, each channel 0..1). `None` for a malformed value or one outside the
/// range, which the portal uses for "no accent".
pub fn parse_accent(v: &Value<'_>) -> Option<Rgb> {
    let Value::Structure(s) = unwrap_variant(v) else {
        return None;
    };
    let [r, g, b] = s.fields() else {
        return None;
    };
    let f = |v: &Value<'_>| match unwrap_variant(v) {
        Value::F64(x) => Some(*x),
        _ => None,
    };
    Rgb::from_unit(f(r)?, f(g)?, f(b)?)
}

/// Turns one portal setting into a change, or `None` for keys kelma does not use.
pub fn parse_change(namespace: &str, key: &str, value: &Value<'_>) -> Option<PortalChange> {
    match (namespace, key) {
        (NS_APPEARANCE, KEY_COLOR_SCHEME) => {
            parse_color_scheme(value).map(PortalChange::ColorScheme)
        }
        // A malformed accent is "no accent", not an ignored event.
        (NS_APPEARANCE, KEY_ACCENT_COLOR) => Some(PortalChange::Accent(parse_accent(value))),
        (NS_APPEARANCE, KEY_CONTRAST) => parse_contrast(value).map(PortalChange::HighContrast),
        (NS_GNOME_INTERFACE, KEY_ENABLE_ANIMATIONS) => {
            parse_bool(value).map(PortalChange::AnimationsEnabled)
        }
        _ => None,
    }
}

// ---- D-Bus --------------------------------------------------------------------------------

/// No session bus, no portal service, or no such key: all mean "the portal has no answer".
fn is_absent(e: &zbus::Error) -> bool {
    match e {
        zbus::Error::MethodError(name, _, _) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.NotSupported"
                | "org.freedesktop.portal.Error.NotFound"
        ),
        zbus::Error::InputOutput(_) | zbus::Error::Address(_) => true,
        _ => false,
    }
}

async fn settings_proxy(conn: &zbus::Connection) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::proxy::Builder::new(conn)
        .destination(PORTAL_DEST)?
        .path(PORTAL_PATH)?
        .interface(SETTINGS_IFACE)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

/// Connects to the session bus; `Ok(None)` when there is none.
async fn connect() -> Result<Option<zbus::Connection>, PortalError> {
    match zbus::Connection::session().await {
        Ok(c) => Ok(Some(c)),
        Err(e) if is_absent(&e) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Reads one setting: `ReadOne` first, falling back to the older `Read` on portals without it.
/// `Ok(None)` when the key does not exist.
pub(crate) async fn read_key(
    proxy: &zbus::Proxy<'_>,
    namespace: &str,
    key: &str,
) -> zbus::Result<Option<OwnedValue>> {
    let first: zbus::Result<OwnedValue> = proxy.call("ReadOne", &(namespace, key)).await;
    let res = match first {
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
        {
            proxy.call("Read", &(namespace, key)).await
        }
        other => other,
    };
    match res {
        Ok(v) => Ok(Some(v)),
        Err(e) if is_absent(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads the current color scheme, accent, contrast and animation switch. `Ok(None)` if there is no session bus or no portal.
pub async fn read() -> Result<Option<PortalSettings>, PortalError> {
    let Some(conn) = connect().await? else {
        return Ok(None);
    };
    let proxy = match settings_proxy(&conn).await {
        Ok(p) => p,
        Err(e) if is_absent(&e) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut out = PortalSettings::default();
    let scheme = read_key(&proxy, NS_APPEARANCE, KEY_COLOR_SCHEME).await;
    match scheme {
        Ok(Some(v)) => out.color_scheme = parse_color_scheme(&v).unwrap_or_default(),
        Ok(None) => {}
        // No portal service on the bus at all.
        Err(e) if is_absent(&e) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    if let Some(v) = read_key(&proxy, NS_APPEARANCE, KEY_ACCENT_COLOR).await? {
        out.accent = parse_accent(&v);
    }
    if let Some(v) = read_key(&proxy, NS_APPEARANCE, KEY_CONTRAST).await? {
        out.high_contrast = parse_contrast(&v).unwrap_or(false);
    }
    if let Some(v) = read_key(&proxy, NS_GNOME_INTERFACE, KEY_ENABLE_ANIMATIONS).await? {
        out.animations_enabled = parse_bool(&v);
    }
    Ok(Some(out))
}

/// Subscribes to `SettingChanged`. `Ok(None)` if there is no session bus. The stream yields only
/// changes to keys kelma uses; call [`read`] first for the starting state.
pub async fn watch() -> Result<Option<PortalStream>, PortalError> {
    let Some(conn) = connect().await? else {
        return Ok(None);
    };
    let proxy = settings_proxy(&conn).await?;
    let signals = proxy.receive_signal("SettingChanged").await?;
    let stream = signals.filter_map(move |msg| {
        // Keep the connection and proxy alive as long as the stream is.
        let _keep = (&conn, &proxy);
        let change = msg
            .body()
            .deserialize::<(String, String, OwnedValue)>()
            .ok()
            .and_then(|(ns, key, v)| parse_change(&ns, &key, &v));
        async move { change }
    });
    Ok(Some(Box::pin(stream)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{StructureBuilder, Value};

    fn owned(v: impl Into<Value<'static>>) -> OwnedValue {
        OwnedValue::try_from(v.into()).unwrap()
    }

    fn rgb_value(r: f64, g: f64, b: f64) -> OwnedValue {
        let s = StructureBuilder::new()
            .add_field(r)
            .add_field(g)
            .add_field(b)
            .build()
            .unwrap();
        owned(Value::Structure(s))
    }

    #[test]
    fn color_scheme_values() {
        assert_eq!(
            parse_color_scheme(&owned(0u32)),
            Some(PortalScheme::NoPreference)
        );
        assert_eq!(parse_color_scheme(&owned(1u32)), Some(PortalScheme::Dark));
        assert_eq!(parse_color_scheme(&owned(2u32)), Some(PortalScheme::Light));
        // Unknown numbers mean no preference.
        assert_eq!(
            parse_color_scheme(&owned(7u32)),
            Some(PortalScheme::NoPreference)
        );
        // Wrong type is not a scheme.
        assert_eq!(parse_color_scheme(&owned("dark")), None);
    }

    #[test]
    fn legacy_read_double_variant_is_unwrapped() {
        let inner = Value::Value(Box::new(Value::U32(1)));
        let v = OwnedValue::try_from(Value::Value(Box::new(inner))).unwrap();
        assert_eq!(parse_color_scheme(&v), Some(PortalScheme::Dark));
    }

    #[test]
    fn accent_in_range() {
        assert_eq!(
            parse_accent(&rgb_value(1.0, 0.0, 0.5)),
            Some(Rgb::new(255, 0, 128))
        );
        assert_eq!(
            parse_accent(&rgb_value(0.0, 0.0, 0.0)),
            Some(Rgb::new(0, 0, 0))
        );
    }

    #[test]
    fn accent_out_of_range_or_malformed_is_none() {
        assert_eq!(parse_accent(&rgb_value(-1.0, 0.0, 0.0)), None);
        assert_eq!(parse_accent(&rgb_value(0.2, 1.5, 0.0)), None);
        assert_eq!(parse_accent(&rgb_value(f64::NAN, 0.0, 0.0)), None);
        assert_eq!(parse_accent(&owned(3u32)), None);
        let two = StructureBuilder::new()
            .add_field(0.1f64)
            .add_field(0.2f64)
            .build()
            .unwrap();
        assert_eq!(parse_accent(&owned(Value::Structure(two))), None);
        let ints = StructureBuilder::new()
            .add_field(1u32)
            .add_field(2u32)
            .add_field(3u32)
            .build()
            .unwrap();
        assert_eq!(parse_accent(&owned(Value::Structure(ints))), None);
    }

    #[test]
    fn changes_are_routed_by_key() {
        assert_eq!(
            parse_change(NS_APPEARANCE, KEY_COLOR_SCHEME, &owned(1u32)),
            Some(PortalChange::ColorScheme(PortalScheme::Dark))
        );
        assert_eq!(
            parse_change(NS_APPEARANCE, KEY_ACCENT_COLOR, &rgb_value(0.0, 1.0, 0.0)),
            Some(PortalChange::Accent(Some(Rgb::new(0, 255, 0))))
        );
        // A bad accent clears it.
        assert_eq!(
            parse_change(NS_APPEARANCE, KEY_ACCENT_COLOR, &rgb_value(9.0, 9.0, 9.0)),
            Some(PortalChange::Accent(None))
        );
        assert_eq!(
            parse_change(NS_APPEARANCE, "unused-key", &owned(1u32)),
            None
        );
        assert_eq!(
            parse_change("org.example", KEY_COLOR_SCHEME, &owned(1u32)),
            None
        );
    }

    #[test]
    fn settings_apply_changes() {
        let mut s = PortalSettings::default();
        s.apply(PortalChange::ColorScheme(PortalScheme::Light));
        s.apply(PortalChange::Accent(Some(Rgb::new(1, 2, 3))));
        assert_eq!(
            s,
            PortalSettings {
                color_scheme: PortalScheme::Light,
                accent: Some(Rgb::new(1, 2, 3)),
                ..Default::default()
            }
        );
        s.apply(PortalChange::Accent(None));
        assert_eq!(s.accent, None);
    }

    #[test]
    fn contrast_values() {
        assert_eq!(parse_contrast(&owned(0u32)), Some(false));
        assert_eq!(parse_contrast(&owned(1u32)), Some(true));
        assert_eq!(parse_contrast(&owned(5u32)), Some(false));
        assert_eq!(parse_contrast(&owned(true)), None);
        assert_eq!(
            parse_change(NS_APPEARANCE, KEY_CONTRAST, &owned(1u32)),
            Some(PortalChange::HighContrast(true))
        );
    }

    #[test]
    fn animation_switch() {
        assert_eq!(parse_bool(&owned(false)), Some(false));
        assert_eq!(parse_bool(&owned(1u32)), None);
        assert_eq!(
            parse_change(NS_GNOME_INTERFACE, KEY_ENABLE_ANIMATIONS, &owned(false)),
            Some(PortalChange::AnimationsEnabled(false))
        );
        // The key only counts in the GNOME namespace.
        assert_eq!(
            parse_change(NS_APPEARANCE, KEY_ENABLE_ANIMATIONS, &owned(false)),
            None
        );
        let mut s = PortalSettings::default();
        s.apply(PortalChange::HighContrast(true));
        s.apply(PortalChange::AnimationsEnabled(false));
        assert!(s.high_contrast);
        assert_eq!(s.animations_enabled, Some(false));
    }

    /// Reads the real portal on this desktop (read-only). Run with
    /// `cargo test -p appearance -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "reads the live desktop portal"]
    async fn live_portal_read() {
        let got = read().await;
        println!("portal read: {got:?}");
        assert!(got.is_ok(), "read must not fail: {got:?}");
    }
}
