//! Teams reaction keys and the "More reactions" picker rows.
//!
//! Teams names six reactions (`like heart laugh surprised sad angry`); every other emoji is
//! `<hex>_<name>` (for example `1f525_fire`), which [`crate::markdown::reaction_emoji`] decodes.

use emoji::Emoji;

/// Most rows the picker shows at once.
pub const PICKER_LIMIT: usize = 96;

/// Teams' own reactions, first in the picker.
const NAMED: [(&str, &str, &str); 6] = [
    ("like", "👍", "like"),
    ("heart", "❤️", "heart"),
    ("laugh", "😆", "laugh"),
    ("surprised", "😮", "surprised"),
    ("sad", "😢", "sad"),
    ("angry", "😡", "angry"),
];

/// One picker cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub key: String,
    pub emoji: String,
    pub name: String,
}

/// Reaction key for an emoji: the Teams name for the six named ones, else `<hex>_<name>`.
/// Variation selectors are left out of the codepoints, as Teams does.
pub fn reaction_key(e: &Emoji) -> String {
    if let Some((key, _, _)) = NAMED.iter().find(|(_, ch, _)| *ch == e.ch) {
        return (*key).to_string();
    }
    let hex: Vec<String> =
        e.ch.chars()
            .filter(|c| *c != '\u{fe0f}')
            .map(|c| format!("{:x}", c as u32))
            .collect();
    let name: String = e
        .name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("")
        .to_lowercase();
    format!("{}_{name}", hex.join("-"))
}

fn named_items() -> impl Iterator<Item = PickerItem> {
    NAMED.iter().map(|(key, ch, name)| PickerItem {
        key: (*key).to_string(),
        emoji: (*ch).to_string(),
        name: (*name).to_string(),
    })
}

fn item(e: &Emoji) -> PickerItem {
    PickerItem {
        key: reaction_key(e),
        emoji: e.ch.to_string(),
        name: e.name.to_string(),
    }
}

/// Picker rows for a search. An empty query lists the Teams reactions, then the emoji table;
/// a query lists the best matches, so the six Teams names (`heart`, `like`, ...) work too.
pub fn picker_items(query: &str) -> Vec<PickerItem> {
    let q = query.trim();
    let mut out: Vec<PickerItem> = Vec::new();
    if q.is_empty() {
        out.extend(named_items());
        out.extend(
            emoji::all()
                .iter()
                .filter(|e| !NAMED.iter().any(|(_, ch, _)| *ch == e.ch))
                .map(item),
        );
    } else {
        out.extend(emoji::search(q, PICKER_LIMIT).into_iter().map(item));
    }
    out.truncate(PICKER_LIMIT);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::reaction_emoji;

    #[test]
    fn named_reactions_use_teams_keys() {
        let heart = emoji::by_shortcode("heart").unwrap();
        assert_eq!(reaction_key(heart), "heart");
        let like = emoji::by_shortcode("like").unwrap();
        assert_eq!(reaction_key(like), "like");
    }

    #[test]
    fn other_emoji_use_codepoint_keys_that_decode_back() {
        let fire = emoji::by_shortcode("fire").unwrap();
        assert_eq!(reaction_key(fire), "1f525_fire");
        assert_eq!(reaction_emoji("1f525_fire"), "🔥");
        let clap = emoji::search("clapping hands", 1)[0];
        assert!(reaction_key(clap).starts_with("1f44f_"));
        assert_eq!(reaction_emoji(&reaction_key(clap)), clap.ch);
    }

    #[test]
    fn variation_selector_is_dropped_from_keys() {
        let sun = emoji::all().iter().find(|e| e.ch == "☀\u{fe0f}").unwrap();
        assert!(reaction_key(sun).starts_with("2600_"));
    }

    #[test]
    fn empty_query_lists_teams_reactions_first() {
        let items = picker_items("");
        let keys: Vec<_> = items.iter().take(6).map(|i| i.key.as_str()).collect();
        assert_eq!(
            keys,
            ["like", "heart", "laugh", "surprised", "sad", "angry"]
        );
        assert_eq!(items.len(), PICKER_LIMIT);
        let named = items.iter().filter(|i| i.key == "like").count();
        assert_eq!(named, 1, "no duplicate of the named reactions");
    }

    #[test]
    fn search_finds_by_name_and_by_teams_alias() {
        let fire = picker_items("fire");
        assert!(fire.iter().any(|i| i.key == "1f525_fire"), "{fire:?}");
        let heart = picker_items("heart");
        assert_eq!(heart[0].key, "heart");
        assert!(picker_items("zzzzqqq").is_empty());
    }
}
