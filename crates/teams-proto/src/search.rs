//! Message search: the Substrate query service behind the Teams search box.
//!
//! It needs `x-anchormailbox` (otherwise it answers 200 with empty sets). People reuse
//! [`crate::people`]; files use the Graph search in the `onedrive` crate (the Substrate `File`
//! entity answered `500` for this account); chats and channels are searched locally.
//! Sources: rusty-teams, m0nkmaster, xteams (see `docs/research/open-source.md` §3.10).

use crate::broker::Audience;
use crate::client::Client;
use crate::client_info::TEAMS_ORIGIN;
use crate::error::{Error, Result};
use crate::model::split_thread;
use crate::people::{anchor_mailbox, oid_of_mri};
use crate::time::parse_iso_ms;
use reqwest::Method;
use serde_json::{Value, json};

const QUERY_URL: &str = "https://substrate.office.com/searchservice/api/v2/query";
const EXT: &str = "Extension_SkypeSpaces_ConversationPost_Extension_";

/// The "In", "Person" and "Date" filters of the Search page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    /// Only this chat or channel (its thread id).
    pub in_thread: Option<String>,
    /// Only messages sent by this person (their MRI, `8:orgid:<oid>`).
    pub from_mri: Option<String>,
    /// Only messages sent on or after this day, `YYYY-MM-DD`.
    pub since_day: Option<String>,
}

/// The query string: the typed text plus the KQL for the filters. Soft-deleted messages are
/// always excluded.
pub fn kql(text: &str, f: &SearchFilters) -> String {
    let mut q = format!("NOT (isClientSoftDeleted:TRUE) AND ({text})");
    if let Some(t) = &f.in_thread {
        q.push_str(&format!(" AND clientthreadid:{t}"));
    }
    if let Some(mri) = &f.from_mri {
        q.push_str(&format!(" AND {EXT}FromSkypeInternalId_String:{mri}"));
    }
    if let Some(day) = &f.since_day {
        q.push_str(&format!(" AND sent>={day}"));
    }
    q
}

/// KQL text that finds the messages mentioning a person (their MRI, `8:orgid:<oid>`).
pub fn mentions_query(mri: &str) -> Option<String> {
    Some(format!("mentions:{}", oid_of_mri(mri)?.replace('-', "")))
}

/// Request body for a message search.
pub fn query_body(
    text: &str,
    filters: &SearchFilters,
    from: u32,
    size: u32,
    cvid: &str,
    logical_id: &str,
) -> Value {
    json!({
        "entityRequests": [{
            "entityType": "Message",
            "contentSources": ["Teams"],
            "propertySet": "Optimized",
            "fields": [
                format!("{EXT}FromSkypeInternalId_String"),
                format!("{EXT}FileData_String"),
                format!("{EXT}ThreadType_String"),
                format!("{EXT}SkypeGroupId_String"),
                format!("{EXT}ParentMessageId_String"),
            ],
            "query": {"queryString": kql(text, filters), "displayQueryString": text},
            "from": from,
            "size": size,
            "topResultsCount": 5
        }],
        "QueryAlterationOptions": {
            "EnableAlteration": true,
            "EnableSuggestion": true,
            "SupportedRecourseDisplayTypes": ["Suggestion"]
        },
        "cvid": cvid,
        "logicalId": logical_id,
        "scenario": {"Name": "powerbar", "Dimensions": [
            {"DimensionName": "QueryType", "DimensionValue": "Messages"},
            {"DimensionName": "FormFactor", "DimensionValue": "general.web.reactSearch"}
        ]},
        "timezone": "UTC"
    })
}

/// One message found by a search.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MessageHit {
    /// The chat or channel (without `;messageid=`).
    pub conversation_id: String,
    /// The message's own id. Live checks showed it is the `ParentMessageId` field (a misleading
    /// name) for chat messages and channel posts, and equals the send time in milliseconds for a
    /// channel reply, where `ParentMessageId` and the id in `ClientConversationId` both name the
    /// thread root. The time has only 10 ms precision on some rows, so for a reply treat it as
    /// close to the id, not equal.
    pub message_id: String,
    /// The thread root of a channel reply, empty for everything else.
    pub root_id: String,
    pub sender_name: String,
    pub sender_mri: String,
    pub time_ms: i64,
    /// `HitHighlightedSummary`, else the preview, as plain text.
    pub snippet: String,
    /// Chat topic, or the channel name for channel posts.
    pub topic: String,
    pub team_name: String,
    /// `chat`, `space`, `topic`, `meeting`, …
    pub thread_type: String,
    /// The message @mentions the signed-in user.
    pub is_mention: bool,
}

/// One page of message hits.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MessagePage {
    pub total: u64,
    pub more: bool,
    pub hits: Vec<MessageHit>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Strips the markup of a hit's summary and decodes the entities the service escapes.
/// Highlight tags (`<c0>…</c0>`) vanish without a trace; any other tag (a mention's `<span>` or
/// `<at>`, a `<p>`) is a word boundary, so two adjacent mentions stay two words. Whitespace runs
/// collapse to one space.
pub fn plain_snippet(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let name = rest[..end].trim_start_matches('/');
        let highlight = name
            .strip_prefix('c')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if !highlight {
            out.push(' ');
        }
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parses the answer to a `Message` query. Rows without a conversation are dropped.
pub fn parse_messages(v: &Value) -> MessagePage {
    let Some(set) = v.pointer("/EntitySets/0/ResultSets/0") else {
        return MessagePage::default();
    };
    let hits = set
        .get("Results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let src = r.get("Source")?;
            let ext = |k: &str| {
                src.pointer(&format!(
                    "/Extensions/SkypeSpaces_ConversationPost_Extension_{k}"
                ))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
            };
            let (conv, _) = split_thread(&s(src, "ClientConversationId"));
            let conversation_id = if conv.is_empty() {
                s(src, "ClientThreadId")
            } else {
                conv
            };
            if conversation_id.is_empty() {
                return None;
            }
            let mut summary = s(r, "HitHighlightedSummary");
            if summary.is_empty() {
                summary = s(src, "Preview");
            }
            let snippet = plain_snippet(&summary);
            let channel = s(src, "ChannelName");
            let time_ms = parse_iso_ms(&s(src, "DateTimeSent")).unwrap_or(0);
            let parent = ext("ParentMessageId");
            let is_reply = conversation_id.ends_with("@thread.tacv2")
                && parent
                    .parse::<i64>()
                    .is_ok_and(|p| (p - time_ms).abs() > 1000);
            let (message_id, root_id) = if is_reply {
                (time_ms.to_string(), parent)
            } else {
                (parent, String::new())
            };
            Some(MessageHit {
                conversation_id,
                message_id,
                root_id,
                sender_name: src
                    .pointer("/From/EmailAddress/Name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                sender_mri: ext("FromSkypeInternalId"),
                time_ms,
                snippet,
                topic: if channel.is_empty() {
                    s(src, "ConversationTopic")
                } else {
                    channel
                },
                team_name: s(src, "TeamName"),
                thread_type: ext("ThreadType"),
                is_mention: src
                    .pointer("/MentionsPreview/IsMentioned")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect();
    MessagePage {
        total: set.get("Total").and_then(Value::as_u64).unwrap_or(0),
        more: set
            .get("MoreResultsAvailable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        hits,
    }
}

impl Client {
    /// Searches messages. `from` and `size` page the results (`size` up to about 50).
    pub async fn search_messages(
        &self,
        self_mri: &str,
        text: &str,
        filters: &SearchFilters,
        from: u32,
        size: u32,
    ) -> Result<MessagePage> {
        let tenant = self.tenant_id().await?;
        let anchor = anchor_mailbox(self_mri, &tenant)
            .ok_or_else(|| Error::Protocol(format!("{self_mri} is not an orgid MRI")))?;
        let cvid = uuid::Uuid::new_v4().to_string();
        let body = query_body(
            text,
            filters,
            from,
            size,
            &cvid,
            &uuid::Uuid::new_v4().to_string(),
        );
        let (status, answer) = self
            .request_as(
                &Audience::Substrate,
                Method::POST,
                QUERY_URL,
                Some(&body),
                &[
                    ("Origin", TEAMS_ORIGIN),
                    ("Referer", &format!("{TEAMS_ORIGIN}/")),
                    ("x-anchormailbox", &anchor),
                    ("x-client-version", "1"),
                    ("client-request-id", &cvid),
                    ("X-ClientType", "MicrosoftTeams"),
                    ("X-ClientScenario", "TeamsSearch"),
                ],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("search", status, &answer));
        }
        Ok(parse_messages(&serde_json::from_str(&answer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_keep_the_space_between_adjacent_mentions() {
        for gap in ["&nbsp;", " ", "\n", "\u{a0}"] {
            let html = format!(
                r#"<p><span itemtype="http://schema.skype.com/Mention" itemid="0">Ana Test</span>{gap}<span itemtype="http://schema.skype.com/Mention" itemid="1">Bo Test</span> <c0>hi</c0></p>"#
            );
            assert_eq!(plain_snippet(&html), "Ana Test Bo Test hi", "gap {gap:?}");
        }
        // Mentions with no whitespace at all in the markup are still two words.
        assert_eq!(
            plain_snippet(r#"<at id="0">Ana Test</at><at id="1">Bo Test</at> hi"#),
            "Ana Test Bo Test hi"
        );
    }

    fn result(conv_ref: &str, parent: &str, summary: &str) -> Value {
        json!({
            "HitHighlightedSummary": summary,
            "Source": {
                "ClientConversationId": conv_ref,
                "ClientThreadId": "19:abc@thread.v2",
                "DateTimeSent": "2026-11-18T10:00:00.500Z",
                "ConversationTopic": "Topic",
                "From": {"EmailAddress": {"Name": "Sara Hassan", "Address": "s@example.com"}},
                "MentionsPreview": {"IsMentioned": true},
                "Extensions": {
                    "SkypeSpaces_ConversationPost_Extension_ParentMessageId": parent,
                    "SkypeSpaces_ConversationPost_Extension_FromSkypeInternalId": "8:orgid:u1",
                    "SkypeSpaces_ConversationPost_Extension_ThreadType": "chat"
                }
            }
        })
    }

    #[test]
    fn kql_adds_the_filters() {
        assert_eq!(
            kql("hello", &SearchFilters::default()),
            "NOT (isClientSoftDeleted:TRUE) AND (hello)"
        );
        let f = SearchFilters {
            in_thread: Some("19:abc@thread.v2".into()),
            from_mri: Some("8:orgid:u1".into()),
            since_day: Some("2026-11-01".into()),
        };
        let q = kql("hello", &f);
        assert!(q.contains(" AND clientthreadid:19:abc@thread.v2"));
        assert!(q.contains("FromSkypeInternalId_String:8:orgid:u1"));
        assert!(q.ends_with(" AND sent>=2026-11-01"));
    }

    #[test]
    fn mentions_query_uses_the_oid_without_dashes() {
        assert_eq!(
            mentions_query("8:orgid:ab12-cd34").as_deref(),
            Some("mentions:ab12cd34")
        );
        assert_eq!(mentions_query("28:bot"), None);
    }

    #[test]
    fn body_carries_text_paging_and_scenario() {
        let b = query_body("hi", &SearchFilters::default(), 25, 25, "c", "l");
        let r = &b["entityRequests"][0];
        assert_eq!(r["entityType"], "Message");
        assert_eq!(r["from"], 25);
        assert_eq!(r["query"]["displayQueryString"], "hi");
        assert_eq!(b["cvid"], "c");
        assert_eq!(b["scenario"]["Dimensions"][0]["DimensionValue"], "Messages");
    }

    #[test]
    fn parses_hits_with_sender_time_and_ids() {
        let v = json!({"EntitySets": [{"ResultSets": [{
            "Total": 7, "MoreResultsAvailable": true,
            "Results": [result("19:abc@thread.v2;messageid=1699999999999", "1795000000500", "see <c0>hello</c0> &amp; more")]
        }]}]});
        let page = parse_messages(&v);
        assert_eq!((page.total, page.more, page.hits.len()), (7, true, 1));
        let h = &page.hits[0];
        assert_eq!(h.conversation_id, "19:abc@thread.v2");
        assert_eq!(h.message_id, "1795000000500");
        assert_eq!(h.sender_name, "Sara Hassan");
        assert_eq!(h.sender_mri, "8:orgid:u1");
        assert_eq!(h.snippet, "see hello & more");
        assert_eq!(h.time_ms % 1000, 500);
        assert!(h.time_ms > 1_700_000_000_000);
        assert!(h.is_mention);
        assert_eq!(h.thread_type, "chat");
    }

    #[test]
    fn rows_without_a_conversation_and_empty_answers_are_dropped() {
        assert_eq!(parse_messages(&json!({})), MessagePage::default());
        let mut r = result("", "", "x");
        r["Source"]["ClientThreadId"] = json!("");
        let v = json!({"EntitySets": [{"ResultSets": [{"Results": [r]}]}]});
        assert!(parse_messages(&v).hits.is_empty());
    }

    #[test]
    fn falls_back_to_the_thread_id_and_the_preview() {
        let mut r = result("", "1700000000001", "");
        r["Source"]["Preview"] = json!("preview text");
        let v = json!({"EntitySets": [{"ResultSets": [{"Results": [r]}]}]});
        let h = &parse_messages(&v).hits[0];
        assert_eq!(h.conversation_id, "19:abc@thread.v2");
        assert_eq!(h.message_id, "1700000000001");
        assert_eq!(h.snippet, "preview text");
    }

    #[test]
    fn channel_posts_and_replies_are_told_apart_by_the_send_time() {
        let chan = "19:chan@thread.tacv2";
        let root = parse_iso_ms("2026-11-18T10:00:00.500Z")
            .unwrap()
            .to_string();
        let root = root.as_str();
        let mut post = result(&format!("{chan};messageid={root}"), root, "p");
        let mut reply = result(&format!("{chan};messageid={root}"), root, "r");
        reply["Source"]["DateTimeSent"] = json!("2026-11-18T10:05:00.250Z");
        post["Source"]["DateTimeSent"] = json!("2026-11-18T10:00:00.500Z");
        for r in [&mut post, &mut reply] {
            r["Source"]["ClientThreadId"] = json!(chan);
        }
        let v = json!({"EntitySets": [{"ResultSets": [{"Results": [post, reply]}]}]});
        let hits = parse_messages(&v).hits;
        assert_eq!(
            (hits[0].message_id.as_str(), hits[0].root_id.as_str()),
            (root, "")
        );
        assert_eq!(hits[1].root_id, root);
        assert_eq!(hits[1].message_id, hits[1].time_ms.to_string());
    }
}
