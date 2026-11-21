//! Teams stickers for the composer's Stickers tab: the catalogue, the images on Teams' public
//! CDN (`statics.teams.cdn.office.net/evergreen-assets/stickerassets`, no sign-in, see
//! docs/research/open-source.md) and the message HTML that sends one. The catalogue
//! (`stickers.tsv`: id, category, file, name) and the HTML shape come from rusty-teams (MIT,
//! `crates/app/assets/stickers.tsv` and `crates/app-base/src/remote_image.rs`).

use std::sync::OnceLock;

const CATALOG: &str = include_str!("stickers.tsv");
const IMAGE_BASE: &str = "https://statics.teams.cdn.office.net/evergreen-assets/stickerassets";
const IMAGE_VERSION: u32 = 5;
/// Stickers are sent as 250 by 250 images.
const SIZE: u32 = 250;
const POPULAR: [&str; 6] = [
    "Clippy_HelloThere",
    "OfficeDrama_HighFive",
    "WordArt_Awesome",
    "Octocorn_Coffee",
    "Teamsquatch_Yes",
    "Bee_Angry",
];

#[derive(Debug, PartialEq, Eq)]
pub struct Sticker {
    pub id: &'static str,
    pub category: &'static str,
    pub file: &'static str,
    pub name: &'static str,
}

impl Sticker {
    pub fn url(&self) -> String {
        format!(
            "{IMAGE_BASE}/{}-{SIZE}x{SIZE}/{}?v={IMAGE_VERSION}",
            self.category.to_lowercase(),
            self.file
        )
    }
}

fn catalog() -> &'static [Sticker] {
    static PARSED: OnceLock<Vec<Sticker>> = OnceLock::new();
    PARSED.get_or_init(|| {
        CATALOG
            .lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                Some(Sticker {
                    id: f.next()?,
                    category: f.next()?,
                    file: f.next()?,
                    name: f.next()?,
                })
            })
            .collect()
    })
}

/// Stickers whose name, id or category has every word of `query`; an empty query gives the
/// popular ones first, then the catalogue in order.
pub fn find(query: &str, limit: usize) -> Vec<&'static Sticker> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        let popular = POPULAR
            .iter()
            .filter_map(|id| catalog().iter().find(|s| s.id == *id));
        let rest = catalog().iter().filter(|s| !POPULAR.contains(&s.id));
        return popular.chain(rest).take(limit).collect();
    }
    catalog()
        .iter()
        .filter(|s| {
            let hay = format!("{} {} {}", s.name, s.id, s.category).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .take(limit)
        .collect()
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The message HTML for a sticker: an image by URL with the Sticker item type (nothing is
/// uploaded). From rusty-teams; not yet checked against a live Teams send by ironteams.
pub fn sticker_html(s: &Sticker) -> String {
    format!(
        r#"<img src="{}" width="{SIZE}" height="{SIZE}" alt="{}" itemtype="http://schema.skype.com/Sticker">"#,
        attr(&s.url()),
        attr(s.name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_of_the_catalogue_parses() {
        assert_eq!(catalog().len(), CATALOG.lines().count());
        assert!(catalog().len() > 400);
    }

    #[test]
    fn the_url_uses_the_lowercase_category_folder() {
        let s = find("hello there", 1)[0];
        assert_eq!(s.id, "Clippy_HelloThere");
        assert_eq!(
            s.url(),
            "https://statics.teams.cdn.office.net/evergreen-assets/stickerassets/clippy-250x250/Clippy_HelloThere.gif?v=5"
        );
    }

    #[test]
    fn an_empty_query_starts_with_the_popular_ones() {
        let found = find("", 10);
        assert_eq!(found.len(), 10);
        assert_eq!(found[0].id, "Clippy_HelloThere");
        assert_eq!(found[5].id, "Bee_Angry");
        assert!(
            find("octocorn coffee", 50)
                .iter()
                .all(|s| s.category == "Octocorn")
        );
        assert!(find("no such sticker", 50).is_empty());
    }

    #[test]
    fn the_html_is_a_square_image_with_the_sticker_itemtype() {
        let html = sticker_html(find("hello there", 1)[0]);
        assert_eq!(
            html,
            r#"<img src="https://statics.teams.cdn.office.net/evergreen-assets/stickerassets/clippy-250x250/Clippy_HelloThere.gif?v=5" width="250" height="250" alt="Hello There" itemtype="http://schema.skype.com/Sticker">"#
        );
    }
}
