//! Pure helpers: time labels, previews, message-row mapping, image decoding.

use crate::data::Pixels;
use chrono::{Datelike, Local, TimeZone};
use serde_json::json;
use store::{ConvKind, MessageRow};
use teams_proto::model::{ConversationKind, Message};

pub(super) fn kind_from(k: ConversationKind) -> Option<ConvKind> {
    Some(match k {
        ConversationKind::OneOnOne => ConvKind::OneOnOne,
        ConversationKind::Group => ConvKind::Group,
        ConversationKind::Channel => ConvKind::Channel,
        ConversationKind::Meeting => ConvKind::Meeting,
        ConversationKind::Notes => ConvKind::Notes,
        ConversationKind::Other => return None,
    })
}

/// Serializes the per-message extras we keep (reactions, files, subject) into the store column.
fn props_json(m: &Message) -> String {
    json!({
        "r": m.reactions.iter().map(|r| json!({"key": r.key, "users": r.users})).collect::<Vec<_>>(),
        "f": m.files.iter().map(|f| json!({"name": f.name, "url": f.url, "type": f.file_type})).collect::<Vec<_>>(),
        "s": m.subject,
        "m": super::mentions::store_value(&m.mentions),
        "c": m.card,
        "t": m.announcement.as_ref().map(|a| a.title.as_str()),
        "i": m.importance.wire(),
        "g": super::media::language_of(m.language_stamp.as_deref()),
        "l": super::media::links_json(&m.links),
    })
    .to_string()
}

pub(super) fn row_from_message(m: &Message, parent: Option<&str>) -> MessageRow {
    MessageRow {
        id: m.id.clone(),
        conversation_id: m.conversation_id.clone(),
        parent_id: parent.map(str::to_owned).or_else(|| m.parent_id.clone()),
        sender_mri: m.sender_mri.clone(),
        sender_name: m.sender_name.clone(),
        compose_ms: m.compose_ms,
        edited_ms: m.edited_ms,
        deleted: m.deleted,
        message_type: m.message_type.clone(),
        content_html: m.content.clone(),
        client_message_id: m.client_message_id.clone(),
        reactions_json: props_json(m),
    }
}

/// Preview text for the chat list.
pub(super) fn preview_of(m: &Message, me: &str) -> String {
    let text = if m.deleted {
        "This message has been deleted.".to_string()
    } else {
        let p = render::plain_text(&render::parse_teams_html(&m.content)).replace('\n', " ");
        if p.trim().is_empty() && !m.files.is_empty() {
            format!("📎 {}", m.files[0].name)
        } else {
            p
        }
    };
    if m.sender_mri == me {
        format!("You: {text}")
    } else {
        text
    }
}

pub(super) fn chat_time(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let now = Local::now();
    let days = now
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    match days {
        0 => t.format(appearance::formats::time_pattern()).to_string(),
        1 => "Yesterday".into(),
        2..=6 => t.format("%a").to_string(),
        _ if t.year() == now.year() => t
            .format(appearance::formats::date_pattern(false))
            .to_string(),
        _ => t
            .format(appearance::formats::date_pattern(true))
            .to_string(),
    }
}

pub(super) fn day_label(ms: i64) -> String {
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let days = Local::now()
        .date_naive()
        .signed_duration_since(t.date_naive())
        .num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => t.format("%A, %B %-d, %Y").to_string(),
    }
}

pub(super) fn same_day(a: i64, b: i64) -> bool {
    match (
        Local.timestamp_millis_opt(a).single(),
        Local.timestamp_millis_opt(b).single(),
    ) {
        (Some(x), Some(y)) => x.date_naive() == y.date_naive(),
        _ => false,
    }
}

/// The two member GUIDs of a 1:1 chat id (`19:<a>_<b>@unq.gbl.spaces`).
pub(super) fn one_on_one_members(id: &str) -> Vec<String> {
    id.strip_prefix("19:")
        .and_then(|s| s.strip_suffix("@unq.gbl.spaces"))
        .map(|s| s.split('_').map(|g| format!("8:orgid:{g}")).collect())
        .unwrap_or_default()
}

pub(super) fn decode_image(bytes: &[u8], max_w: u32, max_h: u32) -> Option<Pixels> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > max_w || img.height() > max_h {
        img.thumbnail(max_w, max_h)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(Pixels::clone_from_slice(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
    ))
}

pub(super) fn system_text(m: &MessageRow) -> Option<String> {
    let t = m.message_type.as_str();
    if t.starts_with("ThreadActivity/") {
        let what = t.trim_start_matches("ThreadActivity/");
        Some(match what {
            "AddMember" | "MemberJoined" => "A member was added to the chat.".into(),
            "DeleteMember" | "MemberLeft" => "A member left the chat.".into(),
            "TopicUpdate" => "The chat name was changed.".into(),
            _ => format!("{what}."),
        })
    } else if t.starts_with("Event/Call") {
        Some("Call".into())
    } else if t.starts_with("RichText/Media_Call") {
        Some("Call recording".into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn stored(importance: Option<&str>) -> Value {
        let mut v = json!({
            "id": "1795001078052",
            "conversationLink": "https://emea.ng.msg.teams.microsoft.com/v1/users/ME/conversations/19:abc@thread.v2",
            "from": "https://emea.ng.msg.teams.microsoft.com/v1/users/ME/contacts/8:orgid:11111111-2222-3333-4444-555555555555",
            "imdisplayname": "Sara",
            "composetime": "2026-11-18T11:24:38.0520781Z",
            "messagetype": "RichText/Html",
            "content": "<p>hi</p>",
            "properties": {}
        });
        if let Some(i) = importance {
            v["properties"]["importance"] = json!(i);
        }
        let m = teams_proto::model::message_from_json(&v, "").unwrap();
        serde_json::from_str(&row_from_message(&m, None).reactions_json).unwrap()
    }

    #[test]
    fn an_incoming_importance_is_kept_with_the_message() {
        assert_eq!(stored(Some("high"))["i"], "high");
        assert_eq!(stored(Some("urgent"))["i"], "urgent");
        assert_eq!(stored(None)["i"], "");
    }
}
