//! Accessibility settings (plan X1.3) kept on this machine, and the pure rules that combine them
//! with what the desktop portal reports.
//!
//! The captions, notification, pointer, TTY and sign-language switches are stored only; the
//! meeting host reads them once calling lands.

use crate::portal::PortalSettings;
use crate::prefs::{AppearancePrefs, FONT_SCALE_MIN, clamp_font_scale};
use crate::store::{lenient, prefs_file};
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "accessibility.json";

/// How much "Larger text" adds on top of the font scale.
pub const LARGER_TEXT_FACTOR: f32 = 1.25;
/// Upper bound of the combined text scale (the largest font scale times the larger-text factor).
pub const EFFECTIVE_SCALE_MAX: f32 = 2.5;

/// A switch that can follow the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoSwitch {
    /// Follow the desktop portal.
    #[default]
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessibilityPrefs {
    /// `Auto` follows the portal `org.freedesktop.appearance` `contrast` key.
    #[serde(deserialize_with = "lenient")]
    pub high_contrast: AutoSwitch,
    /// `Auto` follows the portal `org.gnome.desktop.interface` `enable-animations` key, and is
    /// off when the desktop does not expose it.
    #[serde(deserialize_with = "lenient")]
    pub reduced_motion: AutoSwitch,
    /// Makes text [`LARGER_TEXT_FACTOR`] times bigger on top of the font scale.
    #[serde(deserialize_with = "lenient")]
    pub larger_text: bool,
    #[serde(deserialize_with = "lenient")]
    pub always_show_captions: bool,
    /// Show my name on my own captions.
    #[serde(deserialize_with = "lenient")]
    pub identify_me_in_captions: bool,
    #[serde(deserialize_with = "lenient")]
    pub filter_profanity: bool,
    /// Notification sounds off.
    #[serde(deserialize_with = "lenient")]
    pub muted_notification: bool,
    #[serde(deserialize_with = "lenient")]
    pub single_pointer_controls: bool,
    #[serde(deserialize_with = "lenient")]
    pub tty_mode: bool,
    #[serde(deserialize_with = "lenient")]
    pub sign_language_mode: bool,
}

prefs_file!(AccessibilityPrefs, FILE_NAME);

/// High contrast: `On` and `Off` win; `Auto` follows the portal (off with no portal).
pub fn resolve_high_contrast(prefs: &AccessibilityPrefs, portal: Option<&PortalSettings>) -> bool {
    match prefs.high_contrast {
        AutoSwitch::On => true,
        AutoSwitch::Off => false,
        AutoSwitch::Auto => portal.is_some_and(|p| p.high_contrast),
    }
}

/// Reduced motion: `On` and `Off` win; `Auto` is on only when the desktop says animations are
/// disabled, and off when it says nothing.
pub fn resolve_reduced_motion(prefs: &AccessibilityPrefs, portal: Option<&PortalSettings>) -> bool {
    match prefs.reduced_motion {
        AutoSwitch::On => true,
        AutoSwitch::Off => false,
        AutoSwitch::Auto => portal.is_some_and(|p| p.animations_enabled == Some(false)),
    }
}

/// The text scale to draw with: the font scale, times [`LARGER_TEXT_FACTOR`] when larger text is
/// on, kept within 0.75 to [`EFFECTIVE_SCALE_MAX`].
pub fn effective_font_scale(appearance: &AppearancePrefs, a11y: &AccessibilityPrefs) -> f32 {
    let base = clamp_font_scale(appearance.font_scale);
    let scaled = if a11y.larger_text {
        base * LARGER_TEXT_FACTOR
    } else {
        base
    };
    scaled.clamp(FONT_SCALE_MIN, EFFECTIVE_SCALE_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal(high: bool, anim: Option<bool>) -> PortalSettings {
        PortalSettings {
            high_contrast: high,
            animations_enabled: anim,
            ..Default::default()
        }
    }

    fn a11y(hc: AutoSwitch, rm: AutoSwitch) -> AccessibilityPrefs {
        AccessibilityPrefs {
            high_contrast: hc,
            reduced_motion: rm,
            ..Default::default()
        }
    }

    #[test]
    fn defaults_follow_the_desktop() {
        let d = AccessibilityPrefs::default();
        assert_eq!(d.high_contrast, AutoSwitch::Auto);
        assert_eq!(d.reduced_motion, AutoSwitch::Auto);
        assert!(!d.larger_text && !d.always_show_captions && !d.tty_mode);
    }

    #[test]
    fn high_contrast_resolution() {
        let on = portal(true, None);
        let off = portal(false, None);
        let auto = a11y(AutoSwitch::Auto, AutoSwitch::Auto);
        assert!(resolve_high_contrast(&auto, Some(&on)));
        assert!(!resolve_high_contrast(&auto, Some(&off)));
        assert!(!resolve_high_contrast(&auto, None));
        let forced_on = a11y(AutoSwitch::On, AutoSwitch::Auto);
        assert!(resolve_high_contrast(&forced_on, Some(&off)));
        assert!(resolve_high_contrast(&forced_on, None));
        let forced_off = a11y(AutoSwitch::Off, AutoSwitch::Auto);
        assert!(!resolve_high_contrast(&forced_off, Some(&on)));
    }

    #[test]
    fn reduced_motion_resolution() {
        let auto = a11y(AutoSwitch::Auto, AutoSwitch::Auto);
        // Animations disabled on the desktop: reduce.
        assert!(resolve_reduced_motion(
            &auto,
            Some(&portal(false, Some(false)))
        ));
        // Animations enabled, key missing, or no portal: do not reduce.
        assert!(!resolve_reduced_motion(
            &auto,
            Some(&portal(false, Some(true)))
        ));
        assert!(!resolve_reduced_motion(&auto, Some(&portal(false, None))));
        assert!(!resolve_reduced_motion(&auto, None));
        let on = a11y(AutoSwitch::Auto, AutoSwitch::On);
        assert!(resolve_reduced_motion(
            &on,
            Some(&portal(false, Some(true)))
        ));
        assert!(resolve_reduced_motion(&on, None));
        let off = a11y(AutoSwitch::Auto, AutoSwitch::Off);
        assert!(!resolve_reduced_motion(
            &off,
            Some(&portal(false, Some(false)))
        ));
    }

    #[test]
    fn larger_text_composes_with_font_scale() {
        let mut ap = AppearancePrefs::default();
        let mut ac = AccessibilityPrefs::default();
        assert_eq!(effective_font_scale(&ap, &ac), 1.0);
        ac.larger_text = true;
        assert_eq!(effective_font_scale(&ap, &ac), 1.25);
        ap.font_scale = 1.5;
        assert_eq!(effective_font_scale(&ap, &ac), 1.875);
        ap.font_scale = 2.0;
        assert_eq!(effective_font_scale(&ap, &ac), 2.5);
        // An out-of-range font scale in memory is clamped first.
        ap.font_scale = 9.0;
        assert_eq!(effective_font_scale(&ap, &ac), 2.5);
        ap.font_scale = 0.1;
        ac.larger_text = false;
        assert_eq!(effective_font_scale(&ap, &ac), 0.75);
    }

    #[test]
    fn partial_and_bad_files_load() {
        let p: AccessibilityPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, AccessibilityPrefs::default());
        let p: AccessibilityPrefs =
            serde_json::from_str(r#"{"high_contrast":"on","larger_text":true,"x":1}"#).unwrap();
        assert_eq!(p.high_contrast, AutoSwitch::On);
        assert!(p.larger_text);
        assert_eq!(p.reduced_motion, AutoSwitch::Auto);
        let p: AccessibilityPrefs = serde_json::from_str(
            r#"{"high_contrast":"maybe","reduced_motion":"off","larger_text":"yes","tty_mode":true}"#,
        )
        .unwrap();
        assert_eq!(p.high_contrast, AutoSwitch::Auto);
        assert_eq!(p.reduced_motion, AutoSwitch::Off);
        assert!(!p.larger_text);
        assert!(p.tty_mode);
    }

    #[test]
    fn round_trip_and_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(
            AccessibilityPrefs::load_from(&path),
            AccessibilityPrefs::default()
        );
        let p = AccessibilityPrefs {
            high_contrast: AutoSwitch::On,
            reduced_motion: AutoSwitch::Off,
            larger_text: true,
            always_show_captions: true,
            identify_me_in_captions: true,
            filter_profanity: true,
            muted_notification: true,
            single_pointer_controls: true,
            tty_mode: true,
            sign_language_mode: true,
        };
        p.save_to(&path).unwrap();
        assert_eq!(AccessibilityPrefs::load_from(&path), p);
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(
            AccessibilityPrefs::load_from(&path),
            AccessibilityPrefs::default()
        );
    }
}
