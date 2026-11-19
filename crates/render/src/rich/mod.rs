//! Rich composer model: everything the Teams composer can format, decoded
//! from incoming HTML ([`parse_rich_html`]), encoded to the HTML Teams web
//! sends ([`RichDoc::to_html`]) and built from Markdown-style source
//! ([`markdown_to_rich`]).
//!
//! Unlike [`crate::RichText`] (display model, lists flattened, tables as text)
//! this model keeps headings, nested lists, code languages and tables so a
//! composer can round-trip them. Inline runs reuse [`crate::Span`].

mod decode;
mod encode;

use crate::Span;

pub use decode::parse_rich_html;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RichDoc {
    pub blocks: Vec<RichBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RichBlock {
    /// Empty spans encode as the blank line `<p>&nbsp;</p>`.
    Paragraph(Vec<Span>),
    /// `level` is 1..=3 (Teams maps `h4`..`h6` to level 3).
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    /// `'\n'` in a span is a line break inside the quote.
    Quote(Vec<Span>),
    List(RichList),
    /// `language` is lowercase, `None` for plain text.
    Code {
        language: Option<String>,
        text: String,
    },
    Table(RichTable),
    Rule,
    Image {
        url: String,
        width: Option<u32>,
        height: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RichList {
    pub ordered: bool,
    /// First number of an ordered list when it is not 1.
    pub start: Option<u32>,
    pub items: Vec<ListItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListItem {
    pub spans: Vec<Span>,
    /// Lists nested inside this item.
    pub children: Vec<RichList>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RichTable {
    pub rows: Vec<Vec<TableCell>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    /// `<th>` instead of `<td>`.
    pub header: bool,
    pub colspan: u32,
    pub rowspan: u32,
    /// `'\n'` separates lines (one `<p>` each) inside the cell.
    pub spans: Vec<Span>,
}

impl TableCell {
    pub fn new(spans: Vec<Span>) -> Self {
        Self {
            header: false,
            colspan: 1,
            rowspan: 1,
            spans,
        }
    }
}

/// Markdown-style composer source -> [`RichDoc`]. Syntax is that of
/// [`crate::markdown_to_teams_html`], plus a language after the opening
/// code fence.
pub fn markdown_to_rich(src: &str) -> RichDoc {
    parse_rich_html(&crate::markdown_out::markdown_html(src, true))
}

/// Markdown-style composer source -> the HTML Teams web sends for it.
pub fn markdown_to_composer_html(src: &str) -> String {
    markdown_to_rich(src).to_html()
}

/// Teams' highlight palette: `(itemid slug, fallback color Teams puts in the
/// style)`.
const HIGHLIGHT_PALETTE: [(&str, &str); 8] = [
    ("cranberry", "#DF9299"),
    ("pumpkin", "#F4A593"),
    ("marigold", "#FDD472"),
    ("forest", "#E5F18F"),
    ("seafoam", "#82CDA8"),
    ("lightTeal", "#9DD9DB"),
    ("blue", "#C7D4E8"),
    ("lilac", "#EBD3E1"),
];

const HIGHLIGHT_ITEMTYPE: &str = "http://schema.skype.com/HighlightColor";

/// Strict CSS color -> lowercase `#rrggbb`. Accepts `#rgb`, `#rrggbb`,
/// `rgb(r, g, b)` and `rgba(r, g, b, a)` (alpha dropped; fully transparent is
/// rejected). Anything else (names, `var()`, `url()`, stray declarations) is
/// `None`.
fn parse_css_color(value: &str) -> Option<String> {
    let v = value.trim().to_ascii_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        return match hex.len() {
            6 => Some(v),
            3 => Some(format!(
                "#{}",
                hex.chars().flat_map(|c| [c, c]).collect::<String>()
            )),
            _ => None,
        };
    }
    let args = v
        .strip_prefix("rgba(")
        .or_else(|| v.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = args
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (slot, part) in rgb.iter_mut().zip(&parts) {
        *slot = part.parse::<u8>().ok()?;
    }
    if let Some(alpha) = parts.get(3) {
        let a: f32 = alpha.parse().ok()?;
        if !(a > 0.0 && a <= 1.0) {
            return None;
        }
    }
    Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
}

/// Links the composer and decoder accept.
fn is_safe_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:", "tel:"]
        .iter()
        .any(|p| lower.starts_with(p) && lower.len() > p.len())
}

fn is_http_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["http://", "https://"]
        .iter()
        .any(|p| lower.starts_with(p) && lower.len() > p.len())
}

/// Code block language as it may appear in a `language-xyz` class: lowercase
/// `[a-z0-9+#._-]`, at most 32 chars, never plain text.
fn clean_language(lang: &str) -> Option<String> {
    let l = lang.trim().to_ascii_lowercase();
    let ok = !l.is_empty()
        && l.len() <= 32
        && l.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '.' | '_' | '-'));
    (ok && l != "plaintext" && l != "text").then_some(l)
}
