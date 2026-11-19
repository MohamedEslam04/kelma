//! View data of the app screens: plain structs the UI thread turns into Slint models, and the
//! small formatting helpers the builders share.

use crate::data::Pixels;
use app_store::{Certification, Price, WorksIn};

/// One app in a card grid, the flyout or the Manage list.
#[derive(Clone, Default)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub developer: String,
    pub blurb: String,
    pub meta: String,
    pub icon: Option<Pixels>,
    pub letter: String,
    pub color: u32,
    pub installed: bool,
    pub agent: bool,
    pub update: bool,
    pub pinned: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Nav {
    pub key: String,
    pub label: String,
    pub count: String,
}

impl Nav {
    pub fn new(key: &str, label: &str) -> Nav {
        Nav {
            key: key.into(),
            label: label.into(),
            count: String::new(),
        }
    }
}

/// An app pinned on the rail that kelma has no screen of its own for.
#[derive(Clone, Default)]
pub struct RailItem {
    pub id: String,
    pub name: String,
    pub letter: String,
    pub color: u32,
    pub icon: Option<Pixels>,
    /// The kelma view it opens (`planner`, `workflows`, `onenote`, `lists` or `app`).
    pub view: String,
    /// Pinned by the admin's setup policy: cannot be unpinned.
    pub locked: bool,
}

#[derive(Clone, Default)]
pub struct RailData {
    pub items: Vec<RailItem>,
    pub collapsed: bool,
    pub active: String,
    pub flyout_open: bool,
    pub flyout_query: String,
    pub flyout: Vec<Card>,
    pub flyout_loading: bool,
}

#[derive(Clone, Default)]
pub struct PermItem {
    pub app_id: String,
    pub app_name: String,
    pub perm: String,
    pub label: String,
    pub allowed: bool,
}

/// The app page.
#[derive(Clone, Default)]
pub struct AppPage {
    pub id: String,
    pub name: String,
    pub developer: String,
    pub version: String,
    pub blurb: String,
    pub description: String,
    pub icon: Option<Pixels>,
    pub letter: String,
    pub color: u32,
    pub rating: String,
    pub price: String,
    pub cert: String,
    pub works_in: String,
    pub categories: String,
    pub rsc: Vec<String>,
    pub device: Vec<String>,
    pub targets: Vec<Nav>,
    pub installed: bool,
    pub update: bool,
    pub pinned: bool,
    pub can_pin: bool,
    pub opens_here: bool,
    pub privacy: String,
    pub terms: String,
    pub shots: Vec<Option<Pixels>>,
    pub shot_count: usize,
}

#[derive(Clone, Default)]
pub struct StoreData {
    pub page: String,
    pub title: String,
    pub status: String,
    pub popular: Vec<Card>,
    pub org: Vec<Card>,
    pub cards: Vec<Card>,
    pub featured: Vec<Nav>,
    pub categories: Vec<Nav>,
    pub scope: String,
    pub kind: String,
    pub loading: bool,
    pub app: AppPage,
    pub manage: Vec<Card>,
    pub manage_note: String,
    pub policy: String,
    pub perms: Vec<PermItem>,
    pub host_name: String,
    pub host_blurb: String,
    pub host_link: String,
    pub host_tabs: Vec<Nav>,
}

/// A generic list row.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub meta: String,
    pub badge: String,
    /// `ok`, `bad`, `warn` or empty.
    pub tone: &'static str,
    pub done: bool,
    pub selected: bool,
    pub header: bool,
}

#[derive(Clone, Default)]
pub struct Column {
    pub title: String,
    pub cards: Vec<Row>,
}

#[derive(Clone, Default)]
pub struct TableRow {
    pub id: String,
    pub cells: Vec<String>,
}

#[derive(Clone, Default)]
pub struct TasksData {
    pub nav: Vec<Row>,
    pub tasks: Vec<Row>,
    pub columns: Vec<Column>,
    pub title: String,
    pub board: bool,
    pub can_board: bool,
    pub summary: String,
    pub progress: f32,
    pub status: String,
    pub loading: bool,
}

#[derive(Clone, Default)]
pub struct FlowsData {
    pub flows: Vec<Row>,
    pub runs: Vec<Row>,
    pub environment: String,
    pub selected_name: String,
    pub status: String,
    pub loading: bool,
}

#[derive(Clone, Default)]
pub struct NotesData {
    pub notebooks: Vec<Row>,
    pub sections: Vec<Row>,
    pub pages: Vec<Row>,
    pub page_title: String,
    pub page_text: String,
    pub can_open_web: bool,
    pub status: String,
    pub loading: bool,
}

#[derive(Clone, Default)]
pub struct ListsData {
    pub sites: Vec<Row>,
    pub lists: Vec<Row>,
    pub columns: Vec<String>,
    pub items: Vec<TableRow>,
    pub title: String,
    pub status: String,
    pub loading: bool,
}

// -- helpers -------------------------------------------------------------------------------

/// `#62A0F6` as `0x62A0F6`.
pub fn parse_hex(s: &str) -> Option<u32> {
    let h = s.strip_prefix('#')?;
    (h.len() == 6).then(|| u32::from_str_radix(h, 16).ok())?
}

/// An app tile color: the manifest accent when it is dark enough for white text, else one picked
/// from the name so tiles of different apps differ.
pub fn tile_color(accent: &str, name: &str) -> u32 {
    const PALETTE: [u32; 8] = [
        0x5b5fc7, 0x038387, 0x8764b8, 0xc43e1c, 0x2b579a, 0x498205, 0xb146c2, 0xca5010,
    ];
    if let Some(c) = parse_hex(accent) {
        let (r, g, b) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
        // Perceived brightness below 170 keeps white letters readable.
        if (r * 299 + g * 587 + b * 114) / 1000 < 170 {
            return c;
        }
    }
    let sum: usize = name.bytes().map(usize::from).sum();
    PALETTE[sum % PALETTE.len()]
}

/// The first letter of a name, upper case.
pub fn letter(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// `Personal, Chat, Team, Meeting` for what an app works in.
pub fn works_text(w: WorksIn) -> String {
    let mut s = Vec::new();
    for (on, name) in [
        (w.personal, "Personal"),
        (w.chat, "Chat"),
        (w.team, "Team"),
        (w.meeting, "Meeting"),
    ] {
        if on {
            s.push(name);
        }
    }
    s.join(", ")
}

pub fn price_text(p: Price) -> &'static str {
    match p {
        Price::Free => "Free",
        Price::Paid => "Paid",
        Price::Unknown => "",
    }
}

pub fn cert_text(c: Certification) -> &'static str {
    match c {
        Certification::Certified => "Microsoft 365 certified",
        Certification::PublisherAttested => "Publisher attested",
        Certification::None => "",
    }
}

/// A day as `Nov 20` (the year is added for another year than now).
pub fn day(ms: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    if t.year() == Local::now().year() {
        t.format("%b %-d").to_string()
    } else {
        t.format("%b %-d, %Y").to_string()
    }
}

/// A moment as `Nov 20, 14:03`.
pub fn moment(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%b %-d, %H:%M").to_string())
        .unwrap_or_default()
}

/// A run's length: `3 s`, `2 min`.
pub fn duration(start_ms: i64, end_ms: i64) -> String {
    let s = ((end_ms - start_ms).max(0) / 1000) as u64;
    if s < 90 {
        format!("{s} s")
    } else if s < 5400 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_color_keeps_dark_accents_and_replaces_light_ones() {
        assert_eq!(tile_color("#2b579a", "Word"), 0x2b579a);
        // White would hide the letter: a palette color is picked from the name.
        let c = tile_color("#FFFFFF", "Contoso");
        assert_ne!(c, 0xffffff);
        assert_eq!(c, tile_color("", "Contoso"));
        assert_eq!(parse_hex("62A0F6"), None);
    }

    #[test]
    fn letters_and_texts() {
        assert_eq!(letter("planner"), "P");
        assert_eq!(letter("  "), "?");
        let w = WorksIn {
            personal: true,
            team: true,
            ..Default::default()
        };
        assert_eq!(works_text(w), "Personal, Team");
        assert_eq!(duration(0, 3_000), "3 s");
        assert_eq!(duration(0, 300_000), "5 min");
    }
}
