//! Plain view data of the chat list sections, channel post cards and Adaptive Cards (plan 8.1,
//! 8.4, 8.8). Built by the backend, turned into Slint types on the UI thread.

use crate::data::{ChatData, MsgData, Pixels};

/// One cell of a card's column set.
#[derive(Clone)]
pub struct CardCellData {
    pub text: String,
    pub stretch: f32,
    pub size: f32,
    pub bold: bool,
    pub tone: &'static str,
    pub pixels: Option<Pixels>,
}

/// One row of a drawn Adaptive Card (see `teams_proto::chat_card`).
#[derive(Clone)]
pub struct CardRowData {
    /// `text`, `fact`, `columns`, `image`, `actions` or `banner`.
    pub kind: &'static str,
    /// Markdown.
    pub text: String,
    pub label: String,
    pub size: f32,
    pub bold: bool,
    pub subtle: bool,
    pub tone: &'static str,
    pub band: &'static str,
    pub cells: Vec<CardCellData>,
    /// `(title, url)`; an empty url is a button that cannot run.
    pub buttons: Vec<(String, String)>,
    pub pixels: Option<Pixels>,
    pub image_w: f32,
    pub image_h: f32,
}

/// A section of the chat list.
#[derive(Clone)]
pub struct SectionData {
    pub id: String,
    pub name: String,
    pub expanded: bool,
    pub unread: i32,
    /// Made in kelma, so it can be renamed and deleted there.
    pub local: bool,
    pub items: Vec<ChatData>,
}

/// A channel post with its replies.
#[derive(Clone)]
pub struct PostData {
    pub post: MsgData,
    pub replies: Vec<MsgData>,
    /// Replies not shown yet.
    pub more: i32,
    pub expanded: bool,
    pub following: bool,
}

/// What the list menus need to know about the channel whose menu opened.
#[derive(Clone, Default)]
pub struct ChannelMenuData {
    /// `all`, `feed` or `off`.
    pub level: &'static str,
    pub private: bool,
    pub hidden: bool,
    /// Pinned to the top of its team.
    pub pinned: bool,
}

/// Names and ids of the places "Move to section" offers.
#[derive(Clone)]
pub struct ChoiceData {
    pub id: String,
    pub name: String,
}

/// The list options of the "…" menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListOptions {
    pub previews: bool,
    pub times: bool,
    pub combined: bool,
}

impl Default for ListOptions {
    fn default() -> Self {
        ListOptions {
            previews: true,
            times: true,
            combined: false,
        }
    }
}
