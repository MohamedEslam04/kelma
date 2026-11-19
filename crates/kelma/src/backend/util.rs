//! Pure helpers: time labels, previews, message-row mapping, image decoding.

use crate::data::Pixels;
use appearance::formats::{self, Tz};
use chrono::Datelike;
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
    chat_time_at(
        formats::time_zone(),
        chrono::Utc::now().timestamp_millis(),
        ms,
    )
}

/// `chat_time` for the instant `ms` as seen at `now_ms`, with the days counted in `zone`.
fn chat_time_at(zone: Option<Tz>, now_ms: i64, ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let (Some(t), Some(now)) = (formats::local_in(zone, ms), formats::local_in(zone, now_ms))
    else {
        return String::new();
    };
    let days = now.date().signed_duration_since(t.date()).num_days();
    match days {
        0 => t.format(formats::time_pattern()).to_string(),
        1 => "Yesterday".into(),
        2..=6 => t.format("%a").to_string(),
        _ if t.year() == now.year() => t.format(formats::date_pattern(false)).to_string(),
        _ => t.format(formats::date_pattern(true)).to_string(),
    }
}

pub(super) fn day_label(ms: i64) -> String {
    day_label_at(
        formats::time_zone(),
        chrono::Utc::now().timestamp_millis(),
        ms,
    )
}

fn day_label_at(zone: Option<Tz>, now_ms: i64, ms: i64) -> String {
    let (Some(t), Some(now)) = (formats::local_in(zone, ms), formats::local_in(zone, now_ms))
    else {
        return String::new();
    };
    let days = now.date().signed_duration_since(t.date()).num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => t.format("%A, %B %-d, %Y").to_string(),
    }
}

pub(super) fn same_day(a: i64, b: i64) -> bool {
    same_day_in(formats::time_zone(), a, b)
}

fn same_day_in(zone: Option<Tz>, a: i64, b: i64) -> bool {
    match (formats::local_in(zone, a), formats::local_in(zone, b)) {
        (Some(x), Some(y)) => x.date() == y.date(),
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
        Some(
            match what {
                "AddMember" | "MemberJoined" => "A member was added to the chat.",
                "DeleteMember" | "MemberLeft" => "A member left the chat.",
                "TopicUpdate" | "TopicTopicUpdated" => "The name was changed.",
                "TopicDescriptionUpdated" => "The description was changed.",
                "ThreadArchivalUpdate" => "The team was archived or restored.",
                "PictureUpdate" => "The picture was changed.",
                "RoleUpdate" => "A member's role was changed.",
                // Settings changes ("UpdateFavDefault" and the like) read the same way.
                _ => "The settings were changed.",
            }
            .into(),
        )
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
    use chrono::{TimeZone, Utc};
    use serde_json::{Value, json};

    fn utc_ms(day: u32, h: u32, m: u32) -> i64 {
        Utc.with_ymd_and_hms(2026, 11, day, h, m, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn thread_activity_reads_as_a_sentence() {
        let row = |t: &str| MessageRow {
            id: "m1".into(),
            conversation_id: "19:c@thread.tacv2".into(),
            parent_id: None,
            sender_mri: String::new(),
            sender_name: String::new(),
            compose_ms: 0,
            edited_ms: None,
            deleted: false,
            message_type: t.into(),
            content_html: String::new(),
            client_message_id: None,
            reactions_json: String::new(),
        };
        assert_eq!(
            system_text(&row("ThreadActivity/TopicTopicUpdated")).as_deref(),
            Some("The name was changed.")
        );
        assert_eq!(
            system_text(&row("ThreadActivity/UpdateFavDefault")).as_deref(),
            Some("The settings were changed.")
        );
        assert_eq!(system_text(&row("RichText/Html")), None);
    }

    #[test]
    fn the_chat_list_time_is_in_the_chosen_zone() {
        let cairo = Some(Tz::Africa__Cairo);
        let now = utc_ms(19, 12, 0);
        // 08:00 UTC is 10:00 in Cairo; the clock pattern is the shared one (24 or 12 hour).
        let at = utc_ms(19, 8, 0);
        let utc = chat_time_at(Some(Tz::UTC), now, at);
        let eg = chat_time_at(cairo, now, at);
        assert!(utc.starts_with("08:00") || utc.starts_with("8:00"), "{utc}");
        assert!(eg.starts_with("10:00"), "{eg}");
        assert_eq!(chat_time_at(cairo, now, 0), "");
    }

    #[test]
    fn the_day_of_a_time_follows_the_zone() {
        let cairo = Some(Tz::Africa__Cairo);
        // 23:00 UTC on the 18th is 01:00 on the 19th in Cairo.
        let at = utc_ms(18, 23, 0);
        let now = utc_ms(19, 12, 0);
        assert_eq!(chat_time_at(Some(Tz::UTC), now, at), "Yesterday");
        assert!(chat_time_at(cairo, now, at).contains("1:00"));
        assert_eq!(day_label_at(Some(Tz::UTC), now, at), "Yesterday");
        assert_eq!(day_label_at(cairo, now, at), "Today");
        // Two messages 30 minutes apart either side of UTC midnight.
        let (a, b) = (utc_ms(18, 23, 45), utc_ms(19, 0, 15));
        assert!(!same_day_in(Some(Tz::UTC), a, b));
        assert!(same_day_in(cairo, a, b));
    }

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
