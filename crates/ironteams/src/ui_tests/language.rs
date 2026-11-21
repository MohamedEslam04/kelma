//! The interface language (plan X1.2): selecting Arabic translates the `@tr` strings of the main
//! screens, English (or no selection) keeps the source text, and `Theme.rtl` follows the
//! resolved language. The selection is per window context, so tests do not affect each other.

use super::*;
use crate::Theme;
use appearance::GeneralPrefs;

fn prefs(tag: &str) -> GeneralPrefs {
    GeneralPrefs {
        language: Some(tag.into()),
        ..Default::default()
    }
}

fn label(app: &AppWindow, english: &str) -> ElementHandle {
    find_label(app, english)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no element labelled {english:?}"))
}

fn text_of(e: &ElementHandle) -> String {
    e.accessible_label().expect("accessible label").to_string()
}

#[test]
fn arabic_translates_and_english_restores() {
    let app = app_with_messages(vec![msg("m1", "hi", false)]);
    crate::language::apply(&prefs("en"));

    let menu = label(&app, "Profile menu");
    let profile = label(&app, "Open profile of Me");
    assert_eq!(text_of(&menu), "Profile menu");

    crate::language::apply(&prefs("ar"));
    assert_eq!(text_of(&menu), "قائمة الملف الشخصي");
    // A message with an argument keeps the argument (the name is data, not translated).
    assert_eq!(text_of(&profile), "فتح الملف الشخصي لـ Me");

    crate::language::apply(&prefs("en"));
    assert_eq!(text_of(&menu), "Profile menu");
    assert_eq!(text_of(&profile), "Open profile of Me");
}

#[test]
fn without_a_selection_the_text_stays_english() {
    // Slint picks a bundled language from the system locale by itself, so a machine set to
    // Arabic starts in Arabic: that is the documented "follow the system" behaviour.
    if appearance::system_language().is_some_and(|t| appearance::general::is_rtl(&t)) {
        return;
    }
    let app = app_with_messages(vec![]);
    assert_eq!(text_of(&label(&app, "Profile menu")), "Profile menu");
    assert!(!app.global::<Theme>().get_rtl());
}

#[test]
fn rtl_follows_the_resolved_language() {
    let app = app_with_messages(vec![]);
    let theme = app.global::<Theme>();

    for lang in appearance::available_languages() {
        crate::language::apply(&prefs(lang.tag));
        assert_eq!(theme.get_rtl(), lang.rtl, "Theme.rtl for {}", lang.tag);
    }

    // A regional tag resolves to the offered language, so it is right to left too.
    crate::language::apply(&prefs("ar-EG"));
    assert!(theme.get_rtl());
    crate::language::apply(&prefs("en-GB"));
    assert!(!theme.get_rtl());
}
