//! Pure decisions: which theme, accent and text size to show, given the user's choice and what
//! the desktop portal says. No D-Bus here, so all of it is unit-tested.

use crate::portal::{PortalScheme, PortalSettings};
use crate::prefs::{AppearancePrefs, Rgb, Theme, clamp_font_scale};

/// The theme to actually draw with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedTheme {
    Light,
    Dark,
}

/// `Light` and `Dark` win; `System` follows the portal. With no portal answer, or "no
/// preference", the theme is light.
pub fn resolve_theme(prefs: &AppearancePrefs, portal: Option<PortalScheme>) -> ResolvedTheme {
    match prefs.theme {
        Theme::Light => ResolvedTheme::Light,
        Theme::Dark => ResolvedTheme::Dark,
        Theme::System => match portal {
            Some(PortalScheme::Dark) => ResolvedTheme::Dark,
            Some(PortalScheme::Light | PortalScheme::NoPreference) | None => ResolvedTheme::Light,
        },
    }
}

/// The user's accent if set, else the portal's, else `None` (use kelma's built-in accent).
pub fn resolve_accent(prefs: &AppearancePrefs, portal: Option<Rgb>) -> Option<Rgb> {
    prefs.accent.or(portal)
}

/// The font scale, clamped to the supported range.
pub fn resolve_font_scale(prefs: &AppearancePrefs) -> f32 {
    clamp_font_scale(prefs.font_scale)
}

/// Everything the UI needs, in one value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    pub theme: ResolvedTheme,
    pub accent: Option<Rgb>,
    pub density: crate::prefs::Density,
    pub font_scale: f32,
}

/// Combines the preferences with the portal state (`None` when there is no portal).
pub fn resolve(prefs: &AppearancePrefs, portal: Option<&PortalSettings>) -> Resolved {
    Resolved {
        theme: resolve_theme(prefs, portal.map(|p| p.color_scheme)),
        accent: resolve_accent(prefs, portal.and_then(|p| p.accent)),
        density: prefs.density,
        font_scale: resolve_font_scale(prefs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::Density;

    fn prefs(theme: Theme) -> AppearancePrefs {
        AppearancePrefs {
            theme,
            ..Default::default()
        }
    }

    #[test]
    fn explicit_theme_ignores_the_portal() {
        for p in [
            None,
            Some(PortalScheme::Dark),
            Some(PortalScheme::Light),
            Some(PortalScheme::NoPreference),
        ] {
            assert_eq!(resolve_theme(&prefs(Theme::Light), p), ResolvedTheme::Light);
            assert_eq!(resolve_theme(&prefs(Theme::Dark), p), ResolvedTheme::Dark);
        }
    }

    #[test]
    fn system_theme_follows_the_portal() {
        let s = prefs(Theme::System);
        assert_eq!(
            resolve_theme(&s, Some(PortalScheme::Dark)),
            ResolvedTheme::Dark
        );
        assert_eq!(
            resolve_theme(&s, Some(PortalScheme::Light)),
            ResolvedTheme::Light
        );
        assert_eq!(
            resolve_theme(&s, Some(PortalScheme::NoPreference)),
            ResolvedTheme::Light
        );
        assert_eq!(resolve_theme(&s, None), ResolvedTheme::Light);
    }

    #[test]
    fn accent_user_beats_portal() {
        let mine = Rgb::new(1, 2, 3);
        let theirs = Rgb::new(9, 9, 9);
        let mut p = AppearancePrefs::default();
        assert_eq!(resolve_accent(&p, Some(theirs)), Some(theirs));
        assert_eq!(resolve_accent(&p, None), None);
        p.accent = Some(mine);
        assert_eq!(resolve_accent(&p, Some(theirs)), Some(mine));
    }

    #[test]
    fn resolve_combines_everything() {
        let p = AppearancePrefs {
            density: Density::Compact,
            font_scale: 1.25,
            ..Default::default()
        };
        let portal = PortalSettings {
            color_scheme: PortalScheme::Dark,
            accent: Some(Rgb::new(5, 6, 7)),
        };
        let r = resolve(&p, Some(&portal));
        assert_eq!(r.theme, ResolvedTheme::Dark);
        assert_eq!(r.accent, Some(Rgb::new(5, 6, 7)));
        assert_eq!(r.density, Density::Compact);
        assert_eq!(r.font_scale, 1.25);
        assert_eq!(resolve(&p, None).theme, ResolvedTheme::Light);
    }

    #[test]
    fn font_scale_is_clamped() {
        let p = AppearancePrefs {
            font_scale: 9.0,
            ..Default::default()
        };
        assert_eq!(resolve_font_scale(&p), 2.0);
    }
}
