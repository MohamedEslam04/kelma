//! Adaptive Cards as a structured, flat list of rows (plan 8.8).
//!
//! [`crate::cards`] flattens a card into text for previews. This module keeps its structure so the
//! UI can draw it like Teams does: colored container bands ("AT RISK"), fact sets as two columns,
//! sized and colored text, column sets, images and action buttons. Slint cannot recurse, so the
//! card tree is flattened into [`Row`]s: a container becomes a band color on each of its rows, and
//! a column set becomes one row with a cell per column.
//!
//! Inputs, `Action.Submit` and `Action.Execute` are shown as disabled buttons: sending them
//! needs the bot's own endpoint. `Action.OpenUrl` opens the browser.

use base64::Engine;
use serde_json::Value;

/// `contentType` of the banner an announcement post carries in `properties.cards`.
pub const ANNOUNCEMENT_BANNER: &str =
    "application/vnd.microsoft.teams.messaging-announcementBanner";

/// Text color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Default,
    Dark,
    Light,
    Accent,
    Good,
    Warning,
    Attention,
}

/// Background of a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Band {
    #[default]
    None,
    Emphasis,
    Good,
    Attention,
    Warning,
    Accent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// `label` and `text` side by side.
    Fact,
    Columns,
    Image,
    Actions,
    /// The announcement banner: `text` is the headline, `band` the theme color.
    Banner,
}

/// One column of a column set.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// Markdown.
    pub text: String,
    /// Share of the row's width; `0.0` sizes the column to its text ("auto").
    pub stretch: f32,
    pub size: u8,
    pub bold: bool,
    pub tone: Tone,
    pub image: Option<String>,
}

/// An action button. `url` is `None` for actions kelma cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub title: String,
    pub url: Option<String>,
}

/// One row of a drawn card.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub kind: Kind,
    /// Markdown (`**bold**`, `_italic_`, `[text](url)`).
    pub text: String,
    pub label: String,
    /// Font size in px.
    pub size: u8,
    pub bold: bool,
    pub subtle: bool,
    pub tone: Tone,
    pub band: Band,
    pub cells: Vec<Cell>,
    pub buttons: Vec<Button>,
    pub image: String,
}

impl Row {
    fn new(kind: Kind, band: Band) -> Row {
        Row {
            kind,
            text: String::new(),
            label: String::new(),
            size: 14,
            bold: false,
            subtle: false,
            tone: Tone::Default,
            band,
            cells: Vec::new(),
            buttons: Vec::new(),
            image: String::new(),
        }
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or_default()
}

fn visible(v: &Value) -> bool {
    v.get("isVisible").and_then(Value::as_bool) != Some(false)
}

fn size_px(v: &Value) -> u8 {
    match s(v, "size").to_ascii_lowercase().as_str() {
        "small" => 12,
        "medium" => 16,
        "large" => 20,
        "extralarge" => 26,
        _ => 14,
    }
}

fn tone_of(v: &Value) -> Tone {
    match s(v, "color").to_ascii_lowercase().as_str() {
        "dark" => Tone::Dark,
        "light" => Tone::Light,
        "accent" => Tone::Accent,
        "good" => Tone::Good,
        "warning" => Tone::Warning,
        "attention" => Tone::Attention,
        _ => Tone::Default,
    }
}

fn band_of(style: &str) -> Option<Band> {
    Some(match style.to_ascii_lowercase().as_str() {
        "emphasis" => Band::Emphasis,
        "good" => Band::Good,
        "attention" => Band::Attention,
        "warning" => Band::Warning,
        "accent" => Band::Accent,
        "default" => Band::None,
        _ => return None,
    })
}

fn bold_of(v: &Value) -> bool {
    s(v, "weight").eq_ignore_ascii_case("bolder")
}

fn text_row(v: &Value, band: Band) -> Row {
    let mut r = Row::new(Kind::Text, band);
    r.text = s(v, "text").to_string();
    r.size = size_px(v);
    r.bold = bold_of(v);
    r.subtle = v.get("isSubtle").and_then(Value::as_bool) == Some(true);
    r.tone = tone_of(v);
    r
}

fn rich_text(v: &Value) -> String {
    v.get("inlines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|i| {
            let t = i
                .as_str()
                .or_else(|| i.get("text").and_then(Value::as_str))
                .unwrap_or_default();
            let mut out = t.to_string();
            if bold_of(i) {
                out = format!("**{out}**");
            }
            if i.get("italic").and_then(Value::as_bool) == Some(true) {
                out = format!("_{out}_");
            }
            out
        })
        .collect()
}

fn button(a: &Value) -> Option<Button> {
    if !visible(a) {
        return None;
    }
    let title = s(a, "title");
    let title = if title.is_empty() { "Open" } else { title };
    let url = (s(a, "type") == "Action.OpenUrl")
        .then(|| s(a, "url"))
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .map(str::to_string);
    Some(Button {
        title: title.to_string(),
        url,
    })
}

fn actions_row(actions: &Value, band: Band) -> Option<Row> {
    let buttons: Vec<Button> = actions.as_array()?.iter().filter_map(button).collect();
    (!buttons.is_empty()).then(|| {
        let mut r = Row::new(Kind::Actions, band);
        r.buttons = buttons;
        r
    })
}

/// The text and first image of the items of a column.
fn cell_of(col: &Value) -> Cell {
    let mut texts: Vec<String> = Vec::new();
    let mut image = None;
    let (mut size, mut bold, mut tone) = (14, false, Tone::Default);
    fn collect(
        items: &Value,
        texts: &mut Vec<String>,
        image: &mut Option<String>,
        style: &mut (u8, bool, Tone),
    ) {
        for it in items
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| visible(i))
        {
            match s(it, "type") {
                "TextBlock" => {
                    if texts.is_empty() {
                        *style = (size_px(it), bold_of(it), tone_of(it));
                    }
                    texts.push(s(it, "text").to_string());
                }
                "RichTextBlock" => texts.push(rich_text(it)),
                "Image" => {
                    image.get_or_insert_with(|| s(it, "url").to_string());
                }
                "FactSet" => {
                    for f in it
                        .get("facts")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        texts.push(format!("**{}** {}", s(f, "title"), s(f, "value")));
                    }
                }
                _ => {
                    if let Some(inner) = it.get("items") {
                        collect(inner, texts, image, style);
                    }
                }
            }
        }
    }
    let mut style = (size, bold, tone);
    if let Some(items) = col.get("items") {
        collect(items, &mut texts, &mut image, &mut style);
    }
    (size, bold, tone) = style;
    let stretch = match col.get("width") {
        Some(Value::String(w)) if w.eq_ignore_ascii_case("auto") => 0.0,
        Some(Value::String(w)) if w.eq_ignore_ascii_case("stretch") => 1.0,
        Some(Value::String(w)) => w
            .trim_end_matches("px")
            .parse::<f32>()
            .map_or(1.0, |n| if w.ends_with("px") { 0.0 } else { n }),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(1.0) as f32,
        _ => 1.0,
    };
    Cell {
        text: texts.join("\n\n"),
        stretch,
        size,
        bold,
        tone,
        image,
    }
}

fn walk(items: &Value, band: Band, out: &mut Vec<Row>) {
    for it in items
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| visible(i))
    {
        match s(it, "type") {
            "TextBlock" => out.push(text_row(it, band)),
            "RichTextBlock" => {
                let mut r = text_row(it, band);
                r.text = rich_text(it);
                out.push(r);
            }
            "FactSet" => {
                for f in it
                    .get("facts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let mut r = Row::new(Kind::Fact, band);
                    r.label = s(f, "title").to_string();
                    r.text = s(f, "value").to_string();
                    out.push(r);
                }
            }
            "Image" => {
                let url = s(it, "url");
                if url.starts_with("https://") || url.starts_with("http://") {
                    let mut r = Row::new(Kind::Image, band);
                    r.image = url.to_string();
                    r.text = s(it, "altText").to_string();
                    out.push(r);
                }
            }
            "ColumnSet" => {
                let cols: Vec<&Value> = it
                    .get("columns")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|c| visible(c))
                    .collect();
                // A column set can carry its own style as well.
                let own = band_of(s(it, "style")).unwrap_or(band);
                let mut r = Row::new(Kind::Columns, own);
                r.cells = cols.iter().map(|c| cell_of(c)).collect();
                if !r.cells.is_empty() {
                    out.push(r);
                }
            }
            "Container" => {
                let inner = band_of(s(it, "style")).unwrap_or(band);
                if let Some(items) = it.get("items") {
                    walk(items, inner, out);
                }
            }
            "ActionSet" => {
                if let Some(r) = it.get("actions").and_then(|a| actions_row(a, band)) {
                    out.push(r);
                }
            }
            _ => {}
        }
    }
}

fn adaptive_rows(card: &Value) -> Vec<Row> {
    let mut out = Vec::new();
    if let Some(body) = card.get("body") {
        walk(body, Band::None, &mut out);
    }
    if let Some(r) = card.get("actions").and_then(|a| actions_row(a, Band::None)) {
        out.push(r);
    }
    out
}

fn hero_rows(card: &Value) -> Vec<Row> {
    let mut out = Vec::new();
    for (key, bold) in [("title", true), ("subtitle", false), ("text", false)] {
        let t = s(card, key);
        if !t.is_empty() {
            let mut r = Row::new(Kind::Text, Band::None);
            r.text = t.to_string();
            r.bold = bold;
            out.push(r);
        }
    }
    let buttons: Vec<Button> = card
        .get("buttons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|b| Button {
            title: s(b, "title").to_string(),
            url: Some(s(b, "value").to_string())
                .filter(|u| u.starts_with("https://") || u.starts_with("http://")),
        })
        .collect();
    if !buttons.is_empty() {
        let mut r = Row::new(Kind::Actions, Band::None);
        r.buttons = buttons;
        out.push(r);
    }
    out
}

fn banner_row(content: &Value, headline: &str) -> Row {
    let theme = content
        .pointer("/imageData/colorTheme")
        .or_else(|| content.get("colorTheme"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let band = match theme.to_ascii_lowercase().as_str() {
        "green" | "good" => Band::Good,
        "red" | "attention" => Band::Attention,
        "yellow" | "orange" | "warning" => Band::Warning,
        "blue" | "accent" | "purple" => Band::Accent,
        _ => Band::Emphasis,
    };
    let mut r = Row::new(Kind::Banner, band);
    r.text = headline.to_string();
    r.size = 20;
    r.bold = true;
    r
}

/// The `content` of an attachment: an object, or a JSON string of one.
fn content_of(a: &Value) -> Value {
    match a.get("content") {
        Some(Value::String(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    }
}

/// Rows for a list of attachments (`[{contentType, content}]`). Voice and video cards and
/// anything unknown give no rows: the caller shows the flattened text for those.
/// `headline` is the post's `properties.title`, shown on an announcement banner.
pub fn rows_for_attachments(attachments: &Value, headline: &str) -> Vec<Row> {
    let mut out = Vec::new();
    for a in attachments.as_array().into_iter().flatten() {
        let ct = s(a, "contentType");
        let content = content_of(a);
        if ct == ANNOUNCEMENT_BANNER {
            out.insert(0, banner_row(&content, headline));
        } else if ct.ends_with(".card.adaptive") || ct == "application/vnd.microsoft.card.adaptive"
        {
            out.extend(adaptive_rows(&content));
        } else if ct.ends_with(".card.hero") || ct.ends_with(".card.thumbnail") {
            out.extend(hero_rows(&content));
        }
    }
    out
}

/// The attachments of a `RichText/Media_Card` message: the base64 card in `<Swift b64="…">`.
pub fn swift_attachments(content: &str) -> Option<Value> {
    let start = content.find("b64=\"")? + 5;
    let end = start + content[start..].find('"')?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(content[start..end].trim())
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    Some(v.get("attachments").cloned().unwrap_or(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn adaptive(card: Value) -> Value {
        json!([{"contentType": "application/vnd.microsoft.card.adaptive", "content": card}])
    }

    /// A Workflows-style "release readiness" card: a colored band, a fact set, columns and links.
    fn release_card() -> Value {
        json!({
            "type": "AdaptiveCard",
            "body": [
                {"type": "Container", "style": "attention", "items": [
                    {"type": "TextBlock", "text": "AT RISK", "weight": "Bolder", "color": "Light"},
                    {"type": "TextBlock", "text": "Release readiness", "size": "Large", "weight": "Bolder"}
                ]},
                {"type": "FactSet", "facts": [
                    {"title": "Owner", "value": "kelma"},
                    {"title": "Date", "value": "Friday"}
                ]},
                {"type": "ColumnSet", "columns": [
                    {"type": "Column", "width": "auto", "items": [{"type": "Image", "url": "https://example.com/i.png"}]},
                    {"type": "Column", "width": "stretch", "items": [
                        {"type": "TextBlock", "text": "**3** open items", "size": "Medium", "color": "Warning"}
                    ]}
                ]},
                {"type": "TextBlock", "text": "hidden", "isVisible": false},
                {"type": "ActionSet", "actions": [
                    {"type": "Action.OpenUrl", "title": "Open run", "url": "https://example.com/run/1"},
                    {"type": "Action.Submit", "title": "Approve"}
                ]}
            ],
            "actions": [{"type": "Action.OpenUrl", "title": "Docs", "url": "javascript:alert(1)"}]
        })
    }

    #[test]
    fn container_style_becomes_a_band_on_each_row() {
        let rows = rows_for_attachments(&adaptive(release_card()), "");
        assert_eq!(rows[0].text, "AT RISK");
        assert_eq!(rows[0].band, Band::Attention);
        assert_eq!(rows[0].tone, Tone::Light);
        assert!(rows[0].bold);
        assert_eq!(
            rows[1].band,
            Band::Attention,
            "the whole container is banded"
        );
        assert_eq!(rows[1].size, 20, "size large is 20 px");
        assert_eq!(rows[2].band, Band::None, "rows after the container are not");
    }

    #[test]
    fn fact_sets_are_two_column_rows() {
        let rows = rows_for_attachments(&adaptive(release_card()), "");
        let facts: Vec<_> = rows.iter().filter(|r| r.kind == Kind::Fact).collect();
        assert_eq!(facts.len(), 2);
        assert_eq!(
            (facts[0].label.as_str(), facts[0].text.as_str()),
            ("Owner", "kelma")
        );
    }

    #[test]
    fn column_sets_keep_widths_text_and_images() {
        let rows = rows_for_attachments(&adaptive(release_card()), "");
        let cols = rows.iter().find(|r| r.kind == Kind::Columns).unwrap();
        assert_eq!(cols.cells.len(), 2);
        assert_eq!(cols.cells[0].stretch, 0.0, "auto width");
        assert_eq!(
            cols.cells[0].image.as_deref(),
            Some("https://example.com/i.png")
        );
        assert_eq!(cols.cells[1].stretch, 1.0);
        assert_eq!(cols.cells[1].text, "**3** open items");
        assert_eq!(cols.cells[1].size, 16);
        assert_eq!(cols.cells[1].tone, Tone::Warning);
    }

    #[test]
    fn open_url_buttons_work_and_the_rest_are_disabled() {
        let rows = rows_for_attachments(&adaptive(release_card()), "");
        let sets: Vec<_> = rows.iter().filter(|r| r.kind == Kind::Actions).collect();
        assert_eq!(sets.len(), 2, "the ActionSet and the card's own actions");
        assert_eq!(
            sets[0].buttons[0].url.as_deref(),
            Some("https://example.com/run/1")
        );
        assert_eq!(sets[0].buttons[1].url, None, "Action.Submit cannot run");
        assert_eq!(
            sets[1].buttons[0].url, None,
            "a javascript: url never opens"
        );
        assert!(rows.iter().all(|r| r.text != "hidden"));
    }

    #[test]
    fn hero_cards_and_unknown_cards() {
        let a = json!([{"contentType": "application/vnd.microsoft.card.hero",
            "content": "{\"title\":\"Hi\",\"text\":\"There\",\"buttons\":[{\"title\":\"Go\",\"value\":\"https://x.com\"}]}"}]);
        let rows = rows_for_attachments(&a, "");
        assert_eq!(rows.len(), 3);
        assert!(rows[0].bold);
        assert_eq!(rows[2].buttons[0].url.as_deref(), Some("https://x.com"));
        let audio = json!([{"contentType": "application/vnd.microsoft.card.audio", "content": {}}]);
        assert!(rows_for_attachments(&audio, "").is_empty());
    }

    #[test]
    fn announcement_banner_goes_first_with_the_headline() {
        let a = json!([
            {"contentType": "application/vnd.microsoft.card.adaptive", "content": {"body": [{"type": "TextBlock", "text": "body"}]}},
            {"contentType": ANNOUNCEMENT_BANNER, "content": {"imageData": {"colorTheme": "green"}}}
        ]);
        let rows = rows_for_attachments(&a, "Big news");
        assert_eq!(rows[0].kind, Kind::Banner);
        assert_eq!(rows[0].text, "Big news");
        assert_eq!(rows[0].band, Band::Good);
        assert_eq!(rows[1].text, "body");
    }

    #[test]
    fn swift_content_decodes_to_attachments() {
        let payload = json!({"attachments": adaptive(json!({"body": [{"type": "TextBlock", "text": "hi"}]}))});
        let b64 = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
        let content =
            format!("<URIObject type=\"SWIFT.1\"><Swift b64=\"{b64}\"></Swift></URIObject>");
        let att = swift_attachments(&content).unwrap();
        assert_eq!(rows_for_attachments(&att, "")[0].text, "hi");
        assert!(swift_attachments("<Swift b64=\"!!!\">").is_none());
    }
}
