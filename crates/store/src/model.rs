/// Kind of conversation; stored as lowercase TEXT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvKind {
    OneOnOne,
    Group,
    Channel,
    Meeting,
    Notes,
}

impl ConvKind {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            ConvKind::OneOnOne => "one_on_one",
            ConvKind::Group => "group",
            ConvKind::Channel => "channel",
            ConvKind::Meeting => "meeting",
            ConvKind::Notes => "notes",
        }
    }

    pub(crate) fn from_db(s: &str) -> Option<ConvKind> {
        Some(match s {
            "one_on_one" => ConvKind::OneOnOne,
            "group" => ConvKind::Group,
            "channel" => ConvKind::Channel,
            "meeting" => ConvKind::Meeting,
            "notes" => ConvKind::Notes,
            _ => return None,
        })
    }
}

/// Filter for [`crate::Store::conversations`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvFilter {
    All,
    Unread,
    /// OneOnOne + Group + Meeting + Notes (everything except channels).
    Chats,
    Channels,
    Favorites,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationRow {
    pub id: String,
    pub kind: ConvKind,
    pub title: String,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    pub last_activity_ms: i64,
    pub last_preview: String,
    pub unread: bool,
    pub favorite: bool,
    pub read_horizon_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRow {
    pub id: String,
    pub conversation_id: String,
    /// Channel thread root id; `None` for top-level messages and chats.
    pub parent_id: Option<String>,
    pub sender_mri: String,
    pub sender_name: String,
    pub compose_ms: i64,
    pub edited_ms: Option<i64>,
    pub deleted: bool,
    pub message_type: String,
    pub content_html: String,
    pub client_message_id: Option<String>,
    pub reactions_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UserRow {
    pub mri: String,
    pub display_name: String,
    pub email: Option<String>,
    pub avatar: Option<Vec<u8>>,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TeamRow {
    pub id: String,
    pub name: String,
}
