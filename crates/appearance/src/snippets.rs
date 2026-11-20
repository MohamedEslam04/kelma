//! Snippets: saved quick replies the composer inserts after `/`. They are kept on this machine in
//! `snippets.json`, next to the other preferences: what a person likes to write is theirs, not a
//! Teams account's, and `{first}` and `{me}` are filled from whichever account and chat the
//! snippet is used in.
//!
//! A file that never held a list (`items` missing) shows [`builtin`] examples. Once the list is
//! saved, even an empty one, it is exactly what the user left, so deleted examples stay deleted.

use crate::store::{lenient, prefs_file};
use serde::{Deserialize, Deserializer, Serialize};

pub const FILE_NAME: &str = "snippets.json";

/// One saved reply. `name` is what follows the `/`; `text` may hold several lines and the
/// placeholders `{first}`, `{me}` and `{date}`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snippet {
    pub name: String,
    pub text: String,
}

impl Snippet {
    pub fn new(name: &str, text: &str) -> Snippet {
        Snippet {
            name: name.into(),
            text: text.into(),
        }
    }
}

/// Keeps the entries that have the right shape and a name; one bad entry does not lose the rest.
fn de_items<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Snippet>>, D::Error> {
    let raw: Option<Vec<serde_json::Value>> = lenient(d)?;
    Ok(raw.map(|v| {
        v.into_iter()
            .filter_map(|x| serde_json::from_value::<Snippet>(x).ok())
            .filter(|s| !s.name.is_empty())
            .collect()
    }))
}

/// The saved snippets. `items` is `None` until the user first changes the list.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SnippetPrefs {
    #[serde(deserialize_with = "de_items")]
    pub items: Option<Vec<Snippet>>,
}

/// The examples shown until the list is first saved.
pub fn builtin() -> Vec<Snippet> {
    vec![
        Snippet::new("brb", "Be right back"),
        Snippet::new("omw", "On my way"),
        Snippet::new("thanks", "Thanks, {first}!"),
    ]
}

impl SnippetPrefs {
    /// What the user sees: the saved list, or the examples when none was ever saved.
    pub fn effective(&self) -> Vec<Snippet> {
        self.items.clone().unwrap_or_else(builtin)
    }
}

prefs_file!(SnippetPrefs, FILE_NAME);

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("kelma-snippets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn never_set_shows_the_examples() {
        let p = SnippetPrefs::default();
        assert_eq!(p.items, None);
        assert_eq!(p.effective(), builtin());
        assert_eq!(builtin().len(), 3);
    }

    #[test]
    fn old_and_odd_files_still_load() {
        // No file, an empty object (a file from before the field) and garbage all mean "never set".
        let dir = tmp("old");
        assert_eq!(SnippetPrefs::load_from(&dir.join("none.json")).items, None);
        let empty: SnippetPrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.items, None);
        let wrong: SnippetPrefs = serde_json::from_str(r#"{"items":"x"}"#).unwrap();
        assert_eq!(wrong.items, None);
        // Unknown fields from a newer version are ignored; bad entries are dropped, good ones kept.
        let mixed: SnippetPrefs = serde_json::from_str(
            r#"{"future":1,"items":[{"name":"a","text":"A"},5,{"text":"no name"},{"name":"b"}]}"#,
        )
        .unwrap();
        assert_eq!(
            mixed.items,
            Some(vec![Snippet::new("a", "A"), Snippet::new("b", "")])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_emptied_list_stays_empty_and_round_trips() {
        let dir = tmp("save");
        let path = dir.join(FILE_NAME);
        let p = SnippetPrefs {
            items: Some(vec![]),
        };
        p.save_to(&path).unwrap();
        let back = SnippetPrefs::load_from(&path);
        assert_eq!(back.items, Some(vec![]));
        assert!(back.effective().is_empty(), "deleted examples stay deleted");
        let p = SnippetPrefs {
            items: Some(vec![Snippet::new("hi", "Hello\nthere")]),
        };
        p.save_to(&path).unwrap();
        assert_eq!(SnippetPrefs::load_from(&path), p);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
