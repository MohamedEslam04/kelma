//! General settings (plan X1.2) that live on this machine: app language, date and time format,
//! time zone and the start-up switches. Teams server settings (translation preferences, and the
//! mailbox language and time zone that come from Outlook REST) are not stored here.

use crate::store::{lenient, prefs_file};
use serde::{Deserialize, Deserializer, Serialize};

pub const FILE_NAME: &str = "general.json";

/// A language kelma can show its interface in.
///
/// There is no translation catalogue yet, so the list is static: only English and Arabic
/// (the latter right-to-left). Extend [`available_languages`] when a catalogue lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    /// BCP-47 tag.
    pub tag: &'static str,
    pub native_name: &'static str,
    pub english_name: &'static str,
    pub rtl: bool,
}

static LANGUAGES: [Language; 2] = [
    Language {
        tag: "en",
        native_name: "English",
        english_name: "English",
        rtl: false,
    },
    Language {
        tag: "ar",
        native_name: "العربية",
        english_name: "Arabic",
        rtl: true,
    },
];

/// The language every unmatched request falls back to.
pub const FALLBACK_LANGUAGE: &str = "en";

/// The languages kelma offers.
pub fn available_languages() -> &'static [Language] {
    &LANGUAGES
}

/// Primary subtags written right to left. Works for any tag, offered or not.
pub fn is_rtl(tag: &str) -> bool {
    let primary = tag
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        primary.as_str(),
        "ar" | "he" | "fa" | "ur" | "ps" | "sd" | "yi" | "dv" | "ug" | "ckb"
    )
}

/// Cleans a BCP-47 tag (also accepts `_`): language lower case, script Title case, region upper
/// case. `None` for anything that is not a plausible tag.
pub fn normalize_tag(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 35 {
        return None;
    }
    let mut out = Vec::new();
    for (i, part) in s.split(['-', '_']).enumerate() {
        if part.is_empty() || part.len() > 8 || !part.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        if i == 0 {
            if !(2..=3).contains(&part.len()) || !part.bytes().all(|b| b.is_ascii_alphabetic()) {
                return None;
            }
            out.push(part.to_ascii_lowercase());
        } else if part.len() == 4 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            let mut t = part.to_ascii_lowercase();
            t[..1].make_ascii_uppercase();
            out.push(t);
        } else if part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()) {
            out.push(part.to_ascii_uppercase());
        } else {
            out.push(part.to_ascii_lowercase());
        }
    }
    Some(out.join("-"))
}

/// Turns a POSIX locale such as `ar_EG.UTF-8@euro` into a tag (`ar-EG`). `C`, `POSIX` and
/// empty values mean "no language" and give `None`.
pub fn tag_from_posix(locale: &str) -> Option<String> {
    let base = locale.split(['.', '@']).next().unwrap_or("").trim();
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    normalize_tag(base)
}

/// First usable language from the environment values, in the order given. `LANGUAGE` may hold a
/// colon-separated priority list (`ar:en`); its first usable entry wins.
pub fn system_language_from(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|var| get(var)?.split(':').find_map(tag_from_posix))
}

/// The language the system asks for: `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, then `LANG`.
pub fn system_language() -> Option<String> {
    system_language_from(|k| std::env::var(k).ok())
}

/// Picks the offered language closest to `tag`: an exact match, else the same primary language
/// (`ar-EG` gives `ar`), else English.
pub fn best_match(tag: &str) -> &'static Language {
    let tag = normalize_tag(tag).unwrap_or_default();
    let primary = tag.split('-').next().unwrap_or("");
    let langs = available_languages();
    langs
        .iter()
        .find(|l| l.tag.eq_ignore_ascii_case(&tag))
        .or_else(|| langs.iter().find(|l| l.tag == primary))
        .or_else(|| langs.iter().find(|l| l.tag == FALLBACK_LANGUAGE))
        .unwrap_or(&langs[0])
}

/// The language the interface should use, given the user's choice and the system language
/// (`None` when there is none). Always one of [`available_languages`].
pub fn resolve_language(prefs: &GeneralPrefs, system: Option<&str>) -> &'static Language {
    match prefs.language.as_deref().or(system) {
        Some(tag) => best_match(tag),
        None => best_match(FALLBACK_LANGUAGE),
    }
}

/// [`resolve_language`] with the language read from the environment.
pub fn effective_language(prefs: &GeneralPrefs) -> &'static Language {
    resolve_language(prefs, system_language().as_deref())
}

/// How dates are written. `Auto` follows the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DateFormat {
    #[default]
    Auto,
    /// 31/12/2026
    Dmy,
    /// 12/31/2026
    Mdy,
    /// 2026-12-31
    Ymd,
}

/// 12 or 24 hour clock. `Auto` follows the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeFormat {
    #[default]
    Auto,
    H12,
    H24,
}

fn de_language<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s: Option<String> = lenient(d)?;
    Ok(s.as_deref().and_then(normalize_tag))
}

fn de_time_zone<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s: Option<String> = lenient(d)?;
    Ok(s.map(|z| z.trim().to_string()).filter(|z| !z.is_empty()))
}

/// General preferences kept on this machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralPrefs {
    /// BCP-47 tag for the app language. `None` follows the system.
    #[serde(deserialize_with = "de_language")]
    pub language: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub date_format: DateFormat,
    #[serde(deserialize_with = "lenient")]
    pub time_format: TimeFormat,
    /// IANA zone name (`Africa/Cairo`). `None` follows the system.
    #[serde(deserialize_with = "de_time_zone")]
    pub time_zone: Option<String>,
    /// Start kelma at login. This is the user's intent; the login entry is the tray crate's.
    #[serde(deserialize_with = "lenient")]
    pub auto_start: bool,
    /// Start hidden, in the tray.
    #[serde(deserialize_with = "lenient")]
    pub open_in_background: bool,
    /// Closing the window keeps kelma running in the tray. On by default: that is how kelma
    /// behaved before the switch existed.
    #[serde(deserialize_with = "lenient")]
    pub keep_running_on_close: bool,
}

impl Default for GeneralPrefs {
    fn default() -> Self {
        GeneralPrefs {
            language: None,
            date_format: DateFormat::default(),
            time_format: TimeFormat::default(),
            time_zone: None,
            auto_start: false,
            open_in_background: false,
            keep_running_on_close: true,
        }
    }
}

prefs_file!(GeneralPrefs, FILE_NAME);

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn defaults() {
        let d = GeneralPrefs::default();
        assert_eq!(d.language, None);
        assert_eq!(d.date_format, DateFormat::Auto);
        assert_eq!(d.time_format, TimeFormat::Auto);
        assert_eq!(d.time_zone, None);
        assert!(!d.auto_start && !d.open_in_background);
        assert!(
            d.keep_running_on_close,
            "closing keeps running in the tray, as before"
        );
    }

    #[test]
    fn list_has_english_and_rtl_arabic() {
        let langs = available_languages();
        let en = langs.iter().find(|l| l.tag == "en").unwrap();
        assert_eq!(en.native_name, "English");
        assert!(!en.rtl);
        let ar = langs.iter().find(|l| l.tag == "ar").unwrap();
        assert_eq!(ar.english_name, "Arabic");
        assert_eq!(ar.native_name, "العربية");
        assert!(ar.rtl);
        // Every offered language agrees with the rtl table, and tags are already canonical.
        for l in langs {
            assert_eq!(l.rtl, is_rtl(l.tag), "{}", l.tag);
            assert_eq!(normalize_tag(l.tag).as_deref(), Some(l.tag));
        }
    }

    #[test]
    fn tags_are_normalized() {
        assert_eq!(normalize_tag("AR").as_deref(), Some("ar"));
        assert_eq!(normalize_tag("ar_eg").as_deref(), Some("ar-EG"));
        assert_eq!(normalize_tag("zh-hans-cn").as_deref(), Some("zh-Hans-CN"));
        assert_eq!(normalize_tag(" en-US ").as_deref(), Some("en-US"));
        for bad in ["", "e", "english", "en--US", "en-", "1n", "en US", "en-ü"] {
            assert_eq!(normalize_tag(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn posix_locales() {
        assert_eq!(tag_from_posix("ar_EG.UTF-8").as_deref(), Some("ar-EG"));
        assert_eq!(tag_from_posix("en_US.UTF-8@euro").as_deref(), Some("en-US"));
        assert_eq!(tag_from_posix("fr").as_deref(), Some("fr"));
        assert_eq!(tag_from_posix("C"), None);
        assert_eq!(tag_from_posix("POSIX"), None);
        assert_eq!(tag_from_posix("C.UTF-8"), None);
        assert_eq!(tag_from_posix(""), None);
    }

    #[test]
    fn system_language_uses_the_documented_order() {
        let all = [
            ("LANGUAGE", "ar"),
            ("LC_ALL", "fr_FR.UTF-8"),
            ("LC_MESSAGES", "de_DE.UTF-8"),
            ("LANG", "en_US.UTF-8"),
        ];
        assert_eq!(system_language_from(env(&all)).as_deref(), Some("ar"));
        assert_eq!(
            system_language_from(env(&all[1..])).as_deref(),
            Some("fr-FR")
        );
        assert_eq!(
            system_language_from(env(&all[2..])).as_deref(),
            Some("de-DE")
        );
        assert_eq!(
            system_language_from(env(&all[3..])).as_deref(),
            Some("en-US")
        );
        assert_eq!(system_language_from(env(&[])), None);
    }

    #[test]
    fn system_language_skips_unusable_values() {
        let e = [
            ("LANGUAGE", ""),
            ("LC_ALL", "C"),
            ("LC_MESSAGES", "POSIX"),
            ("LANG", "ar_EG.UTF-8"),
        ];
        assert_eq!(system_language_from(env(&e)).as_deref(), Some("ar-EG"));
        // A priority list: the first usable entry wins.
        let e = [("LANGUAGE", "C:ar_SA:en")];
        assert_eq!(system_language_from(env(&e)).as_deref(), Some("ar-SA"));
    }

    #[test]
    fn best_match_falls_back_gracefully() {
        assert_eq!(best_match("ar").tag, "ar");
        assert_eq!(best_match("ar-EG").tag, "ar");
        assert_eq!(best_match("AR_sa").tag, "ar");
        assert_eq!(best_match("en-GB").tag, "en");
        assert_eq!(best_match("ja").tag, "en");
        assert_eq!(best_match("garbage!!").tag, "en");
    }

    #[test]
    fn effective_language_prefers_the_users_choice() {
        let follow = GeneralPrefs::default();
        assert_eq!(resolve_language(&follow, Some("ar-EG")).tag, "ar");
        assert_eq!(resolve_language(&follow, Some("ja-JP")).tag, "en");
        assert_eq!(resolve_language(&follow, None).tag, "en");
        let mine = GeneralPrefs {
            language: Some("en".into()),
            ..Default::default()
        };
        assert_eq!(resolve_language(&mine, Some("ar")).tag, "en");
        let arabic = GeneralPrefs {
            language: Some("ar".into()),
            ..Default::default()
        };
        assert!(resolve_language(&arabic, Some("en")).rtl);
    }

    #[test]
    fn rtl_table() {
        assert!(is_rtl("ar"));
        assert!(is_rtl("ar-EG"));
        assert!(is_rtl("he_IL"));
        assert!(is_rtl("FA"));
        assert!(!is_rtl("en"));
        assert!(!is_rtl(""));
    }

    #[test]
    fn partial_and_bad_files_load() {
        let p: GeneralPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, GeneralPrefs::default());
        let p: GeneralPrefs =
            serde_json::from_str(r#"{"language":"ar_eg","time_format":"h24","future":true}"#)
                .unwrap();
        assert_eq!(p.language.as_deref(), Some("ar-EG"));
        assert_eq!(p.time_format, TimeFormat::H24);
        let p: GeneralPrefs = serde_json::from_str(
            r#"{"language":"not a tag","date_format":"weird","time_zone":"  ","auto_start":"yes","keep_running_on_close":true}"#,
        )
        .unwrap();
        assert_eq!(p.language, None);
        assert_eq!(p.date_format, DateFormat::Auto);
        assert_eq!(p.time_zone, None);
        assert!(!p.auto_start);
        assert!(p.keep_running_on_close);
    }

    #[test]
    fn round_trip_and_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        assert_eq!(GeneralPrefs::load_from(&path), GeneralPrefs::default());
        let p = GeneralPrefs {
            language: Some("ar".into()),
            date_format: DateFormat::Ymd,
            time_format: TimeFormat::H12,
            time_zone: Some("Africa/Cairo".into()),
            auto_start: true,
            open_in_background: true,
            keep_running_on_close: true,
        };
        p.save_to(&path).unwrap();
        assert_eq!(GeneralPrefs::load_from(&path), p);
        std::fs::write(&path, b"{{{").unwrap();
        assert_eq!(GeneralPrefs::load_from(&path), GeneralPrefs::default());
    }
}
