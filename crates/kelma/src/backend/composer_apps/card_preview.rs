//! What a result card of a messaging extension shows in the picker's result list: a title, a
//! subtitle and a thumbnail address.
//!
//! `app_store::bots` already reads the `title`, `subtitle`/`text` and first image of hero,
//! thumbnail and similar cards. An Adaptive Card has none of those keys, so its first two text
//! blocks and its first image stand in for them.

use app_store::bots::ResultItem;
use serde_json::Value;

/// Longest title or subtitle shown, in characters.
const MAX_CHARS: usize = 120;

/// One row of the result list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub title: String,
    pub subtitle: String,
    /// Where the thumbnail comes from, if the card has one.
    pub image: Option<String>,
}

/// One line of at most [`MAX_CHARS`] characters: whitespace collapsed, cut with an ellipsis.
fn tidy(s: &str) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_CHARS {
        return one_line;
    }
    let cut: String = one_line.chars().take(MAX_CHARS - 1).collect();
    format!("{cut}…")
}

/// The text blocks (document order) and the first image of an Adaptive Card.
fn walk<'a>(v: &'a Value, texts: &mut Vec<&'a str>, image: &mut Option<&'a str>) {
    match v {
        Value::Array(list) => {
            for item in list {
                walk(item, texts, image);
            }
        }
        Value::Object(o) => {
            match o.get("type").and_then(Value::as_str) {
                Some("TextBlock") => {
                    if let Some(t) = o.get("text").and_then(Value::as_str)
                        && !t.trim().is_empty()
                    {
                        texts.push(t);
                    }
                }
                Some("Image") if image.is_none() => {
                    *image = o
                        .get("url")
                        .and_then(Value::as_str)
                        .filter(|u| !u.is_empty());
                }
                _ => {}
            }
            for key in ["body", "items", "columns"] {
                if let Some(child) = o.get(key) {
                    walk(child, texts, image);
                }
            }
        }
        _ => {}
    }
}

/// The preview of a result card.
pub fn preview_of(item: &ResultItem) -> Preview {
    let mut texts = Vec::new();
    let mut image = None;
    walk(&item.content, &mut texts, &mut image);
    let title = if item.title.trim().is_empty() {
        texts.first().map(|t| tidy(t)).unwrap_or_default()
    } else {
        tidy(&item.title)
    };
    let subtitle = if item.subtitle.trim().is_empty() {
        // The text block after the one the title came from.
        let at = usize::from(item.title.trim().is_empty());
        texts.get(at).map(|t| tidy(t)).unwrap_or_default()
    } else {
        tidy(&item.subtitle)
    };
    Preview {
        title: if title.is_empty() {
            "Card".to_owned()
        } else {
            title
        },
        subtitle,
        image: item
            .image
            .clone()
            .or_else(|| image.map(str::to_owned))
            .filter(|u| !u.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(content: Value, title: &str, subtitle: &str, image: Option<&str>) -> ResultItem {
        ResultItem {
            content_type: "application/vnd.microsoft.card.adaptive".into(),
            content,
            title: title.into(),
            subtitle: subtitle.into(),
            image: image.map(str::to_owned),
        }
    }

    #[test]
    fn a_hero_card_keeps_the_title_subtitle_and_image_the_parser_found() {
        let p = preview_of(&item(
            json!({"title": "Cat"}),
            "Cat typing",
            "2 MB",
            Some("https://img.example.invalid/c.gif"),
        ));
        assert_eq!(p.title, "Cat typing");
        assert_eq!(p.subtitle, "2 MB");
        assert_eq!(
            p.image.as_deref(),
            Some("https://img.example.invalid/c.gif")
        );
    }

    #[test]
    fn an_adaptive_card_uses_its_first_two_text_blocks_and_first_image() {
        let card = json!({"type": "AdaptiveCard", "body": [
            {"type": "ColumnSet", "columns": [
                {"type": "Column", "items": [{"type": "Image", "url": "https://img.example.invalid/a.png"}]},
                {"type": "Column", "items": [
                    {"type": "TextBlock", "text": "TCK-1 Login fails"},
                    {"type": "TextBlock", "text": "Open, assigned to Sam"},
                ]},
            ]},
            {"type": "TextBlock", "text": "third"},
        ]});
        let p = preview_of(&item(card, "", "", None));
        assert_eq!(p.title, "TCK-1 Login fails");
        assert_eq!(p.subtitle, "Open, assigned to Sam");
        assert_eq!(
            p.image.as_deref(),
            Some("https://img.example.invalid/a.png")
        );
    }

    #[test]
    fn a_known_title_is_not_repeated_as_the_subtitle() {
        let card = json!({"body": [
            {"type": "TextBlock", "text": "first"},
            {"type": "TextBlock", "text": "second"},
        ]});
        let p = preview_of(&item(card, "Given", "", None));
        assert_eq!(p.title, "Given");
        assert_eq!(
            p.subtitle, "first",
            "texts fill the missing subtitle in order"
        );
    }

    #[test]
    fn a_card_with_nothing_to_show_is_called_a_card() {
        let p = preview_of(&item(
            json!({"type": "AdaptiveCard", "body": []}),
            "",
            "",
            None,
        ));
        assert_eq!(p.title, "Card");
        assert_eq!(p.subtitle, "");
        assert_eq!(p.image, None);
    }

    #[test]
    fn long_and_multi_line_text_is_cut_to_one_line() {
        let long = format!("a\n  b {}", "x".repeat(200));
        let p = preview_of(&item(Value::Null, &long, "", None));
        assert!(p.title.starts_with("a b xxx"), "{}", p.title);
        assert_eq!(p.title.chars().count(), MAX_CHARS);
        assert!(p.title.ends_with('…'));
    }

    #[test]
    fn empty_image_addresses_are_ignored() {
        let card = json!({"body": [{"type": "Image", "url": ""}]});
        assert_eq!(preview_of(&item(card, "t", "", Some(""))).image, None);
    }
}
