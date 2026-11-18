//! Plain `Send` view data built by the backend and turned into Slint types on the UI thread.

use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Decoded RGBA pixels (shared, cheap to clone).
pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

#[derive(Clone)]
pub struct ChatData {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub time: String,
    pub unread: bool,
    pub initials: String,
    pub color: u32,
    pub avatar: Option<Pixels>,
    pub section: &'static str,
    pub rtl_title: bool,
    pub rtl_preview: bool,
    pub favorite: bool,
    /// `preview` is an unsent draft, not the last message.
    pub draft: bool,
}

/// A chip above the composer.
pub struct AttachmentData {
    pub name: String,
    pub detail: String,
    pub is_image: bool,
    pub thumb: Option<Pixels>,
}

#[derive(Clone)]
pub struct ImgData {
    pub url: String,
    pub w: f32,
    pub h: f32,
    pub pixels: Option<Pixels>,
}

#[derive(Clone)]
pub struct FileData {
    pub name: String,
    pub url: String,
    pub ext: String,
}

#[derive(Clone)]
pub struct ReactData {
    pub key: String,
    pub emoji: String,
    pub count: i32,
    pub mine: bool,
}

#[derive(Clone)]
pub struct MsgData {
    pub id: String,
    pub sender: String,
    pub initials: String,
    pub color: u32,
    pub avatar: Option<Pixels>,
    pub mine: bool,
    pub time: String,
    pub day: String,
    pub show_header: bool,
    pub edited: bool,
    pub deleted: bool,
    pub system: bool,
    pub subject: String,
    pub quote_author: String,
    pub quote_text: String,
    pub markdown: String,
    pub plain: String,
    pub rtl: bool,
    pub images: Vec<ImgData>,
    pub files: Vec<FileData>,
    pub reactions: Vec<ReactData>,
    pub reply_count: i32,
    pub pending: bool,
}

#[derive(Clone)]
pub struct TeamData {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub color: u32,
    pub expanded: bool,
    pub channels: Vec<(String, String, bool)>,
}

#[derive(Clone, Default)]
pub struct HeaderData {
    pub id: String,
    pub title: String,
    pub initials: String,
    pub color: u32,
    pub avatar: Option<Pixels>,
    pub is_channel: bool,
    pub thread_root: String,
}

/// Initials for an avatar placeholder ("Sara Hassan" → "SH", "مجموعة الإدارة" → "ما").
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split_whitespace()
        .filter(|w| w.chars().next().is_some_and(char::is_alphanumeric))
        .collect();
    match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect::<String>().to_uppercase(),
        [first, .., last] => {
            let a = first.chars().next().unwrap_or('?');
            let b = last.chars().next().unwrap_or('?');
            format!("{a}{b}").to_uppercase()
        }
    }
}

/// Stable placeholder color for an id.
pub fn color_for(id: &str) -> u32 {
    const PALETTE: [u32; 8] = [
        0x5b5fc7, 0xc4314b, 0x0f7b5f, 0xca5010, 0x8764b8, 0x038387, 0x986f0b, 0x4f6bed,
    ];
    let h = id
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    PALETTE[(h % PALETTE.len() as u32) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_work() {
        assert_eq!(initials("Sara Hassan"), "SH");
        assert_eq!(initials("Omar"), "OM");
        assert_eq!(initials("Mohamed Eslam (You)"), "ME");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("مجموعة الإدارة"), "ما");
    }

    #[test]
    fn colors_are_stable() {
        assert_eq!(color_for("a"), color_for("a"));
    }
}
