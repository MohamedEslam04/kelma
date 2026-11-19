//! The interface language the user picked in Settings (plan X1.2), as a shared setting the UI
//! layer reads when it switches its translation and its reading direction.
//!
//! Settings calls [`set_from`] at start and on every change, like [`crate::formats`]. The
//! result is always one of [`crate::available_languages`].

use crate::general::{FALLBACK_LANGUAGE, GeneralPrefs, Language, best_match, resolve_language};
use std::sync::RwLock;

static CURRENT: RwLock<Option<&'static Language>> = RwLock::new(None);

/// Sets the language from the saved preferences and the system language (`None` when the
/// system gives none).
pub fn set_from_with(prefs: &GeneralPrefs, system: Option<&str>) -> &'static Language {
    let lang = resolve_language(prefs, system);
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(lang);
    lang
}

/// [`set_from_with`] with the system language read from the environment.
pub fn set_from(prefs: &GeneralPrefs) -> &'static Language {
    set_from_with(prefs, crate::general::system_language().as_deref())
}

/// The language in use now (English until [`set_from`] runs).
pub fn current() -> &'static Language {
    CURRENT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_else(|| best_match(FALLBACK_LANGUAGE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(tag: Option<&str>) -> GeneralPrefs {
        GeneralPrefs {
            language: tag.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn follows_the_choice_then_the_system() {
        // One test mutates the shared setting, so the assertions run in order here.
        let l = set_from_with(&prefs(Some("ar")), Some("en"));
        assert_eq!((l.tag, l.rtl), ("ar", true));
        assert_eq!(current().tag, "ar");

        let l = set_from_with(&prefs(None), Some("ar-EG"));
        assert_eq!((l.tag, l.rtl), ("ar", true), "system Arabic is followed");

        let l = set_from_with(&prefs(Some("en")), Some("ar"));
        assert_eq!((l.tag, l.rtl), ("en", false), "an explicit choice wins");

        let l = set_from_with(&prefs(None), Some("fr"));
        assert_eq!(l.tag, "en", "an unoffered language falls back to English");
    }
}
