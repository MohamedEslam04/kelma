//! Global search across messages, files, people and calendar events (plan X3.4).
//!
//! One Substrate `searchservice/api/v2/query` call carries an `entityRequests` entry per kind,
//! so a single round trip answers every tab of the search box. Per-kind paging uses plain
//! `from` offsets: [`SearchResults::more`] holds the next offset of each kind that has more
//! hits and can be passed back as [`SearchOptions::from`].
//!
//! Wire facts: the request shape follows the Teams web client (`powerbar` scenario, `All`
//! query type) as documented in `docs/research/open-source.md` §3.10, with the entity requests
//! adapted from rusty-teams `substrate_search/request.rs` (MIT, (c) 2026 Daniel Daeschle). The
//! service answers 200 with empty sets unless `x-anchormailbox` names our own mailbox.

use crate::broker::Audience;
use crate::client::Client;
use crate::client_info::TEAMS_ORIGIN;
use crate::error::{Error, Result};
use crate::people::{anchor_mailbox, oid_of_mri};
use crate::time::parse_iso_ms;
use reqwest::Method;
use serde_json::{Value, json};
use std::ops::Range;

const QUERY_URL: &str = "https://substrate.office.com/searchservice/api/v2/query";

/// A kind of search hit. The order is the order of the tabs in the Teams search box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchKind {
    Messages,
    Files,
    People,
    Events,
}

impl SearchKind {
    pub const ALL: [SearchKind; 4] = [
        SearchKind::Messages,
        SearchKind::Files,
        SearchKind::People,
        SearchKind::Events,
    ];

    /// The name used on the command line and in the UI tab bar.
    pub fn name(self) -> &'static str {
        match self {
            SearchKind::Messages => "messages",
            SearchKind::Files => "files",
            SearchKind::People => "people",
            SearchKind::Events => "events",
        }
    }

    pub fn from_name(s: &str) -> Option<SearchKind> {
        SearchKind::ALL.into_iter().find(|k| k.name() == s)
    }
}

/// Where each kind's next page starts (the number of hits already seen). `None` means the
/// first page when used as a request, and "no more hits" when returned in a result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Paging {
    pub messages: Option<u32>,
    pub files: Option<u32>,
    pub people: Option<u32>,
    pub events: Option<u32>,
}

impl Paging {
    pub fn get(&self, kind: SearchKind) -> Option<u32> {
        match kind {
            SearchKind::Messages => self.messages,
            SearchKind::Files => self.files,
            SearchKind::People => self.people,
            SearchKind::Events => self.events,
        }
    }

    fn set(&mut self, kind: SearchKind, next: Option<u32>) {
        match kind {
            SearchKind::Messages => self.messages = next,
            SearchKind::Files => self.files = next,
            SearchKind::People => self.people = next,
            SearchKind::Events => self.events = next,
        }
    }

    /// Whether any kind has another page.
    pub fn any(&self) -> bool {
        SearchKind::ALL.into_iter().any(|k| self.get(k).is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// The kinds to search; empty means all of them.
    pub kinds: Vec<SearchKind>,
    /// Hits asked for per kind and page.
    pub page_size: u32,
    /// The start of each kind's page, from a previous [`SearchResults::more`].
    pub from: Paging,
}

impl Default for SearchOptions {
    fn default() -> Self {
        SearchOptions {
            kinds: vec![],
            page_size: 10,
            from: Paging::default(),
        }
    }
}

impl SearchOptions {
    fn kinds(&self) -> Vec<SearchKind> {
        if self.kinds.is_empty() {
            SearchKind::ALL.to_vec()
        } else {
            SearchKind::ALL
                .into_iter()
                .filter(|k| self.kinds.contains(k))
                .collect()
        }
    }
}

/// Text with the ranges (byte offsets) the service marked as matching the query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    pub highlights: Vec<Range<usize>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sender {
    pub name: Option<String>,
    pub email: Option<String>,
    pub mri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageHit {
    pub conversation_id: String,
    /// The message id inside the conversation (the arrival time in milliseconds).
    pub message_id: String,
    /// The id of the thread's first message when the hit is a reply.
    pub root_message_id: Option<String>,
    pub sender: Sender,
    pub snippet: Snippet,
    pub time_ms: Option<i64>,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHit {
    pub name: String,
    pub url: Option<String>,
    /// The folder the file is in.
    pub parent_url: Option<String>,
    /// SharePoint ids that locate the drive item: site, web, list and the item's unique id.
    pub site_id: Option<String>,
    pub web_id: Option<String>,
    pub list_id: Option<String>,
    pub item_id: Option<String>,
    pub extension: Option<String>,
    pub size: Option<u64>,
    pub modified_ms: Option<i64>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonHit {
    pub mri: String,
    pub name: String,
    pub email: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventHit {
    /// The Exchange item id.
    pub id: String,
    pub subject: String,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub organizer: Option<String>,
    pub cancelled: bool,
}

/// A hit of any kind, by position in its list in [`SearchResults`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitRef {
    pub kind: SearchKind,
    pub index: usize,
}

/// Every kind's hits in the service's own ranking, plus the paging state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub messages: Vec<MessageHit>,
    pub files: Vec<FileHit>,
    pub people: Vec<PersonHit>,
    pub events: Vec<EventHit>,
    /// What the service says it holds in all, per kind, for "N results" labels.
    pub totals: [Option<u64>; 4],
    /// The next page of each kind that has one.
    pub more: Paging,
}

impl SearchResults {
    pub fn len_of(&self, kind: SearchKind) -> usize {
        match kind {
            SearchKind::Messages => self.messages.len(),
            SearchKind::Files => self.files.len(),
            SearchKind::People => self.people.len(),
            SearchKind::Events => self.events.len(),
        }
    }

    pub fn total_of(&self, kind: SearchKind) -> Option<u64> {
        self.totals[SearchKind::ALL.iter().position(|k| *k == kind).unwrap_or(0)]
    }

    /// Adds the hits of `other` after ours (paging state and totals are not touched).
    pub fn append(&mut self, other: SearchResults) {
        self.messages.extend(other.messages);
        self.files.extend(other.files);
        self.people.extend(other.people);
        self.events.extend(other.events);
    }

    /// Drops the hits that are already in `earlier`. The service pulls its "top results" of
    /// the first page out of the list, so they come again on a later page.
    pub fn dedup_against(&mut self, earlier: &SearchResults) {
        self.messages.retain(|m| {
            !earlier
                .messages
                .iter()
                .any(|e| (&e.conversation_id, &e.message_id) == (&m.conversation_id, &m.message_id))
        });
        self.files.retain(|f| {
            !earlier
                .files
                .iter()
                .any(|e| (&e.url, &e.item_id, &e.name) == (&f.url, &f.item_id, &f.name))
        });
        self.people
            .retain(|p| !earlier.people.iter().any(|e| e.mri == p.mri));
        self.events
            .retain(|v| !earlier.events.iter().any(|e| e.id == v.id));
    }

    /// The best hits of every kind in one list ("Top results"): the first hit of each kind
    /// (people, files, messages, events), then the second of each, and so on, so that each
    /// kind keeps the service's order and no kind buries the others.
    pub fn top_results(&self, n: usize) -> Vec<HitRef> {
        const ORDER: [SearchKind; 4] = [
            SearchKind::People,
            SearchKind::Files,
            SearchKind::Messages,
            SearchKind::Events,
        ];
        let deepest = ORDER.iter().map(|k| self.len_of(*k)).max().unwrap_or(0);
        (0..deepest)
            .flat_map(|index| {
                ORDER
                    .into_iter()
                    .filter(move |k| index < self.len_of(*k))
                    .map(move |kind| HitRef { kind, index })
            })
            .take(n)
            .collect()
    }
}

const FIELDS_MESSAGE: [&str; 8] = [
    "Extension_SkypeSpaces_ConversationPost_Extension_FromSkypeInternalId_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_FileData_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_ThreadType_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_SkypeGroupId_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_SenderTenantId_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_ParentMessageId_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_ImageSrc_String",
    "Extension_SkypeSpaces_ConversationPost_Extension_AmsReferences_StringArray",
];
const FIELDS_PEOPLE: [&str; 15] = [
    "Id",
    "MRI",
    "DisplayName",
    "EmailAddresses",
    "PeopleType",
    "PeopleSubtype",
    "UserPrincipalName",
    "GivenName",
    "Surname",
    "CompanyName",
    "JobTitle",
    "ImAddress",
    "ExternalDirectoryObjectId",
    "Department",
    "OfficeLocation",
];
/// Hits of the first message page the service ranks as "top results".
const TOP_MESSAGES: u32 = 5;
/// The service answers an event query asking for more than 8 hits with every occurrence of
/// every recurring series it knows (hundreds), so event pages are capped.
const MAX_EVENTS: u32 = 8;

fn people_filter() -> Value {
    let any = |field: &str, values: &[&str]| {
        let terms: Vec<Value> = values.iter().map(|v| json!({"Term": {field: v}})).collect();
        json!({"Or": terms})
    };
    json!({"And": [
        any("PeopleType", &["Person", "Other"]),
        any(
            "PeopleSubtype",
            &["OrganizationUser", "MTOUser", "PersonalContact", "Guest"],
        ),
        any("Flags", &["NonHidden"]),
    ]})
}

/// The JSON body for one query that asks for every kind in `opts`.
pub fn query_body(query: &str, opts: &SearchOptions, cvid: &str, logical_id: &str) -> Value {
    let kinds = opts.kinds();
    let plain = json!({"queryString": query, "displayQueryString": query});
    let entities: Vec<Value> = kinds
        .iter()
        .map(|kind| {
            let from = opts.from.get(*kind).unwrap_or(0);
            let mut e = match kind {
                SearchKind::Messages => {
                    let mut e = json!({
                        "entityType": "Message",
                        "contentSources": ["Teams"],
                        "propertySet": "Optimized",
                        "fields": FIELDS_MESSAGE,
                        "query": {
                            "queryString": format!("NOT (isClientSoftDeleted:TRUE) AND {query}"),
                            "displayQueryString": query,
                        },
                    });
                    if from == 0 {
                        e["topResultsCount"] = json!(TOP_MESSAGES);
                    }
                    e
                }
                SearchKind::Files => json!({
                    "entityType": "File",
                    "contentSources": ["OneDriveBusiness", "Exchange"],
                    "query": plain,
                }),
                SearchKind::People => json!({
                    "entityType": "People",
                    "contentSources": ["Exchange"],
                    "Fields": FIELDS_PEOPLE,
                    "Filter": people_filter(),
                    "query": plain,
                }),
                SearchKind::Events => json!({
                    "entityType": "Event",
                    "contentSources": ["Exchange"],
                    "query": plain,
                }),
            };
            e["from"] = json!(from);
            e["size"] = json!(match kind {
                SearchKind::Events => opts.page_size.min(MAX_EVENTS),
                _ => opts.page_size,
            });
            e
        })
        .collect();
    let query_type = match kinds.as_slice() {
        [SearchKind::Messages] => "Messages",
        [SearchKind::Files] => "Files",
        [SearchKind::People] => "People",
        _ => "All",
    };
    json!({
        "entityRequests": entities,
        "QueryAlterationOptions": {
            "EnableAlteration": true,
            "EnableSuggestion": true,
            "SupportedRecourseDisplayTypes": ["Suggestion"],
        },
        "scenario": {
            "Name": "powerbar",
            "Dimensions": [
                {"DimensionName": "QueryType", "DimensionValue": query_type},
                {"DimensionName": "FormFactor", "DimensionValue": "general.web.reactSearch"},
            ],
        },
        "cvid": cvid,
        "logicalId": logical_id,
        "timezone": "UTC",
    })
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Splits a Substrate hit summary (`<c0>match</c0>`, `<ddd/>` for an elided part) into plain
/// text and the byte ranges of the matches. Entities in the text are decoded.
pub fn parse_highlights(summary: &str) -> Snippet {
    let mut out = Snippet::default();
    let mut open: Option<usize> = None;
    let mut rest = summary;
    while !rest.is_empty() {
        let next = rest.find('<').unwrap_or(rest.len());
        out.text.push_str(&decode_entities(&rest[..next]));
        rest = &rest[next..];
        if rest.is_empty() {
            break;
        }
        if let Some(r) = rest.strip_prefix("<c0>") {
            open.get_or_insert(out.text.len());
            rest = r;
        } else if let Some(r) = rest.strip_prefix("</c0>") {
            if let Some(start) = open.take().filter(|s| *s < out.text.len()) {
                out.highlights.push(start..out.text.len());
            }
            rest = r;
        } else if let Some(r) = rest.strip_prefix("<ddd/>") {
            out.text.push_str("...");
            rest = r;
        } else {
            out.text.push('<');
            rest = &rest[1..];
        }
    }
    out
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn time(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_str).and_then(parse_iso_ms)
}

fn email_name(v: &Value) -> Option<String> {
    text(v.get("EmailAddress")?, "Name")
}

fn message_hit(r: &Value) -> Option<MessageHit> {
    let src = r.get("Source")?;
    let conversation_id = text(src, "ClientThreadId")?;
    let message_id = text(src, "InternetMessageId")?;
    let ext = |name: &str| {
        src.get("Extensions")
            .and_then(|e| text(e, &format!("SkypeSpaces_ConversationPost_Extension_{name}")))
    };
    let sender = src.get("From").or_else(|| src.get("Sender"));
    let snippet = match text(r, "HitHighlightedSummary") {
        Some(s) => parse_highlights(&s),
        None => Snippet {
            text: text(src, "Preview").unwrap_or_default(),
            highlights: vec![],
        },
    };
    Some(MessageHit {
        root_message_id: text(src, "ParentMessageId")
            .or_else(|| ext("ParentMessageId"))
            .or_else(|| {
                text(src, "ClientConversationId")
                    .and_then(|c| c.split_once(";messageid=").map(|(_, id)| id.to_owned()))
            }),
        sender: Sender {
            name: sender.and_then(email_name),
            email: sender
                .and_then(|s| s.get("EmailAddress"))
                .and_then(|e| text(e, "Address")),
            mri: sender
                .and_then(|s| text(s, "MRI"))
                .or_else(|| ext("FromSkypeInternalId")),
        },
        snippet,
        time_ms: time(src, "DateTimeSent"),
        has_attachments: src
            .get("HasAttachments")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        conversation_id,
        message_id,
    })
}

fn file_hit(r: &Value) -> Option<FileHit> {
    let src = r.get("Source")?;
    Some(FileHit {
        name: text(src, "Filename").or_else(|| text(src, "Title"))?,
        url: text(src, "Path").or_else(|| text(src, "OriginalPath")),
        parent_url: text(src, "ParentLink"),
        site_id: text(src, "SiteId"),
        web_id: text(src, "WebId"),
        list_id: text(src, "ListID"),
        item_id: text(src, "UniqueId"),
        extension: text(src, "FileExtension"),
        size: src.get("Size").and_then(Value::as_u64),
        modified_ms: time(src, "LastModifiedTime"),
        author: text(src, "Author"),
    })
}

fn person_hit(r: &Value) -> Option<PersonHit> {
    let src = r.get("Source")?;
    let mri = text(src, "MRI")
        .or_else(|| text(src, "ExternalDirectoryObjectId").map(|oid| format!("8:orgid:{oid}")))?;
    let name = text(src, "DisplayName").or_else(|| {
        let full = format!(
            "{} {}",
            text(src, "GivenName").unwrap_or_default(),
            text(src, "Surname").unwrap_or_default()
        );
        Some(full.trim().to_owned()).filter(|n| !n.is_empty())
    })?;
    Some(PersonHit {
        email: src
            .get("EmailAddresses")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| text(src, "UserPrincipalName")),
        title: text(src, "JobTitle"),
        mri,
        name,
    })
}

fn event_hit(r: &Value) -> Option<EventHit> {
    let src = r.get("Source")?;
    Some(EventHit {
        id: text(src, "Id").or_else(|| src.get("ItemId").and_then(|i| text(i, "Id")))?,
        subject: text(src, "Subject")?,
        start_ms: time(src, "Start"),
        end_ms: time(src, "End"),
        organizer: src.get("Organizer").and_then(email_name),
        cancelled: src
            .get("IsCancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Parses a query response. Unknown kinds and malformed hits are skipped.
pub fn parse_results(v: &Value, opts: &SearchOptions) -> SearchResults {
    let mut out = SearchResults::default();
    for set in v
        .get("EntitySets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let kind = match set.get("EntityType").and_then(Value::as_str) {
            Some("Message") => SearchKind::Messages,
            Some("File") => SearchKind::Files,
            Some("People") => SearchKind::People,
            Some("Event") => SearchKind::Events,
            _ => continue,
        };
        let (mut returned, mut more, mut total) = (0u32, false, None::<u64>);
        for rs in set
            .get("ResultSets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            more |= rs
                .get("MoreResultsAvailable")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            total = total.max(rs.get("Total").and_then(Value::as_u64));
            for r in rs
                .get("Results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                returned += 1;
                match kind {
                    SearchKind::Messages => out.messages.extend(message_hit(r)),
                    SearchKind::Files => out.files.extend(file_hit(r)),
                    SearchKind::People => out.people.extend(person_hit(r)),
                    SearchKind::Events => out.events.extend(event_hit(r)),
                }
            }
        }
        if let Some(i) = SearchKind::ALL.iter().position(|k| *k == kind) {
            out.totals[i] = total;
        }
        let next = opts.from.get(kind).unwrap_or(0) + returned;
        out.more.set(kind, (more && returned > 0).then_some(next));
    }
    out
}

impl Client {
    /// Searches messages, files, people and events for `query` in one request.
    /// `self_mri` is our own MRI (`8:orgid:<oid>`).
    pub async fn global_search(
        &self,
        self_mri: &str,
        query: &str,
        opts: &SearchOptions,
    ) -> Result<SearchResults> {
        let v = self.global_search_json(self_mri, query, opts).await?;
        Ok(parse_results(&v, opts))
    }

    /// The raw JSON answer of [`Client::global_search`], for protocol discovery.
    pub async fn global_search_json(
        &self,
        self_mri: &str,
        query: &str,
        opts: &SearchOptions,
    ) -> Result<Value> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Value::Null);
        }
        let tenant = self.tenant_id().await?;
        let oid = oid_of_mri(self_mri)
            .ok_or_else(|| Error::Protocol(format!("{self_mri} is not an orgid MRI")))?;
        let anchor = anchor_mailbox(self_mri, &tenant).unwrap_or_default();
        let cvid = uuid::Uuid::new_v4().to_string();
        let body = query_body(query, opts, &cvid, &uuid::Uuid::new_v4().to_string());
        let (status, text) = self
            .request_as(
                &Audience::Substrate,
                Method::POST,
                QUERY_URL,
                Some(&body),
                &[
                    ("Origin", TEAMS_ORIGIN),
                    ("Referer", &format!("{TEAMS_ORIGIN}/")),
                    ("x-anchormailbox", &anchor),
                    (
                        "x-routingparameter-sessionkey",
                        &format!("OID:{oid}@{tenant}"),
                    ),
                    ("client-request-id", &uuid::Uuid::new_v4().to_string()),
                    ("client-session-id", &cvid),
                    ("x-client-version", "1"),
                    ("X-ClientType", "MicrosoftTeams"),
                    ("X-ClientScenario", "TeamsSearch"),
                ],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("global search", status, &text));
        }
        Ok(serde_json::from_str(&text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(kinds: &[SearchKind]) -> SearchOptions {
        SearchOptions {
            kinds: kinds.to_vec(),
            ..SearchOptions::default()
        }
    }

    #[test]
    fn body_fans_out_one_entity_request_per_kind() {
        let b = query_body("report", &opts(&[]), "cv", "lg");
        let types: Vec<&str> = b["entityRequests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["entityType"].as_str().unwrap())
            .collect();
        assert_eq!(types, ["Message", "File", "People", "Event"]);
        assert_eq!(b["cvid"], "cv");
        assert_eq!(b["logicalId"], "lg");
        assert_eq!(b["scenario"]["Name"], "powerbar");
        assert_eq!(b["entityRequests"][0]["contentSources"], json!(["Teams"]));
        assert_eq!(
            b["entityRequests"][0]["query"]["queryString"],
            "NOT (isClientSoftDeleted:TRUE) AND report"
        );
        assert_eq!(
            b["entityRequests"][0]["query"]["displayQueryString"],
            "report"
        );
        assert_eq!(b["entityRequests"][1]["query"]["queryString"], "report");
        assert_eq!(
            b["entityRequests"][1]["contentSources"],
            json!(["OneDriveBusiness", "Exchange"])
        );
        assert_eq!(
            b["entityRequests"][2]["contentSources"],
            json!(["Exchange"])
        );
        assert!(b["entityRequests"][2]["Fields"].as_array().unwrap().len() > 5);
    }

    #[test]
    fn body_honours_kinds_page_size_and_offsets() {
        let o = SearchOptions {
            kinds: vec![SearchKind::Files, SearchKind::Messages],
            page_size: 7,
            from: Paging {
                messages: Some(14),
                files: None,
                ..Paging::default()
            },
        };
        let b = query_body("x", &o, "c", "l");
        let e = b["entityRequests"].as_array().unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!(e[0]["entityType"], "Message");
        assert_eq!(
            (e[0]["from"].as_u64(), e[0]["size"].as_u64()),
            (Some(14), Some(7))
        );
        assert_eq!(
            (e[1]["from"].as_u64(), e[1]["size"].as_u64()),
            (Some(0), Some(7))
        );
        assert_eq!(b["scenario"]["Dimensions"][0]["DimensionValue"], "All");
    }

    #[test]
    fn single_kind_body_uses_that_kinds_query_type() {
        let b = query_body("x", &opts(&[SearchKind::People]), "c", "l");
        assert_eq!(b["scenario"]["Dimensions"][0]["DimensionValue"], "People");
        let b = query_body("x", &opts(&[SearchKind::Messages]), "c", "l");
        assert_eq!(b["scenario"]["Dimensions"][0]["DimensionValue"], "Messages");
    }

    #[test]
    fn top_results_only_on_the_first_message_page() {
        let first = query_body("x", &opts(&[SearchKind::Messages]), "c", "l");
        assert!(
            first["entityRequests"][0]["topResultsCount"]
                .as_u64()
                .unwrap()
                > 0
        );
        let next = SearchOptions {
            from: Paging {
                messages: Some(10),
                ..Paging::default()
            },
            ..opts(&[SearchKind::Messages])
        };
        let second = query_body("x", &next, "c", "l");
        assert!(second["entityRequests"][0].get("topResultsCount").is_none());
    }

    #[test]
    fn highlights_become_byte_ranges_over_plain_text() {
        let s = parse_highlights("<ddd/>Grüße an <c0>Max</c0> &amp; <c0>Anna</c0> today");
        assert_eq!(s.text, "...Grüße an Max & Anna today");
        let words: Vec<&str> = s.highlights.iter().map(|r| &s.text[r.clone()]).collect();
        assert_eq!(words, ["Max", "Anna"]);
    }

    #[test]
    fn unclosed_or_empty_highlight_is_tolerated() {
        let s = parse_highlights("a <c0></c0>b <c0>c");
        assert_eq!(s.text, "a b c");
        assert!(s.highlights.is_empty());
        assert_eq!(parse_highlights(""), Snippet::default());
    }

    fn response() -> Value {
        json!({"EntitySets": [
          {"EntityType": "Message", "ResultSets": [{"Total": 120, "MoreResultsAvailable": true, "Results": [
            {"Rank": 1, "HitHighlightedSummary": "weekly <c0>meeting</c0> notes", "Source": {
              "ClientThreadId": "19:abc@thread.v2",
              "ClientConversationId": "19:abc@thread.v2;messageid=1760500000000",
              "InternetMessageId": "1760600000000", "DateTimeSent": "2026-10-16T05:00:00Z",
              "Preview": "weekly meeting notes", "HasAttachments": true,
              "From": {"EmailAddress": {"Name": "Max Muster", "Address": "max@contoso.example"}},
              "Extensions": {"SkypeSpaces_ConversationPost_Extension_FromSkypeInternalId": "8:orgid:aaaa"}}},
            {"Rank": 2, "Source": {"ClientThreadId": "48:notes", "InternetMessageId": "1760700000000",
              "Preview": "plain preview", "Sender": {"EmailAddress": {"Name": "Me"}}}},
            {"Rank": 3, "Source": {"InternetMessageId": "no thread"}}
          ]}]},
          {"EntityType": "File", "ResultSets": [{"Total": 20, "MoreResultsAvailable": false, "Results": [
            {"Rank": 9, "Source": {"Filename": "Plan.docx", "Title": "Plan", "FileExtension": "docx", "Size": 42170,
              "Path": "https://t.sharepoint.example/sites/a/Plan.docx", "ParentLink": "https://t.sharepoint.example/sites/a",
              "SiteId": "s1", "WebId": "w1", "ListID": "l1", "UniqueId": "u1",
              "Author": "Sara", "LastModifiedTime": "2026-10-01T08:00:00Z"}},
            {"Rank": 10, "Source": {"Title": "Only a title", "Path": "https://t.sharepoint.example/x"}},
            {"Rank": 11, "Source": {"Size": 1}}
          ]}]},
          {"EntityType": "People", "ResultSets": [{"Total": 2, "Results": [
            {"Source": {"DisplayName": "Sara Hassan", "EmailAddresses": ["sara@contoso.example"], "JobTitle": "Engineer",
              "ExternalDirectoryObjectId": "oid-a"}},
            {"Source": {"DisplayName": "Omar", "MRI": "8:orgid:oid-b", "UserPrincipalName": "omar@contoso.example"}},
            {"Source": {"DisplayName": "No account"}}
          ]}]},
          {"EntityType": "Event", "ResultSets": [{"Total": 30, "MoreResultsAvailable": true, "Results": [
            {"Source": {"Id": "AAMk1", "Subject": "Release sync", "Start": "2026-10-20T09:00:00Z", "End": "2026-10-20T09:30:00Z",
              "Organizer": {"EmailAddress": {"Name": "Erika", "Address": "e@contoso.example"}}, "IsCancelled": true}},
            {"Source": {"ItemId": {"Id": "AAMk2"}, "Subject": "Standup"}}
          ]}]},
          {"EntityType": "Chat", "ResultSets": [{"Results": [{"Source": {"ThreadId": "x"}}]}]}
        ]})
    }

    #[test]
    fn parses_every_kind_in_service_order() {
        let r = parse_results(&response(), &SearchOptions::default());
        assert_eq!(r.messages.len(), 2);
        let m = &r.messages[0];
        assert_eq!(m.conversation_id, "19:abc@thread.v2");
        assert_eq!(m.message_id, "1760600000000");
        assert_eq!(m.root_message_id.as_deref(), Some("1760500000000"));
        assert_eq!(m.sender.name.as_deref(), Some("Max Muster"));
        assert_eq!(m.sender.email.as_deref(), Some("max@contoso.example"));
        assert_eq!(m.sender.mri.as_deref(), Some("8:orgid:aaaa"));
        assert_eq!(m.snippet.text, "weekly meeting notes");
        assert_eq!(&m.snippet.text[m.snippet.highlights[0].clone()], "meeting");
        assert_eq!(m.time_ms, parse_iso_ms("2026-10-16T05:00:00Z"));
        assert!(m.has_attachments);
        // No summary: the preview is the snippet, with nothing highlighted.
        assert_eq!(r.messages[1].snippet.text, "plain preview");
        assert!(r.messages[1].snippet.highlights.is_empty());
        assert_eq!(r.messages[1].sender.name.as_deref(), Some("Me"));

        assert_eq!(r.files.len(), 2);
        let f = &r.files[0];
        assert_eq!(f.name, "Plan.docx");
        assert_eq!(
            f.url.as_deref(),
            Some("https://t.sharepoint.example/sites/a/Plan.docx")
        );
        assert_eq!(f.item_id.as_deref(), Some("u1"));
        assert_eq!(
            (
                f.site_id.as_deref(),
                f.web_id.as_deref(),
                f.list_id.as_deref()
            ),
            (Some("s1"), Some("w1"), Some("l1"))
        );
        assert_eq!(f.size, Some(42170));
        assert_eq!(f.author.as_deref(), Some("Sara"));
        assert_eq!(r.files[1].name, "Only a title");

        assert_eq!(r.people.len(), 2);
        assert_eq!(r.people[0].mri, "8:orgid:oid-a");
        assert_eq!(r.people[0].title.as_deref(), Some("Engineer"));
        assert_eq!(r.people[1].email.as_deref(), Some("omar@contoso.example"));

        assert_eq!(r.events.len(), 2);
        assert_eq!(r.events[0].id, "AAMk1");
        assert_eq!(r.events[0].organizer.as_deref(), Some("Erika"));
        assert_eq!(r.events[0].end_ms, parse_iso_ms("2026-10-20T09:30:00Z"));
        assert!(r.events[0].cancelled);
        assert_eq!(r.events[1].id, "AAMk2");
        assert!(!r.events[1].cancelled);
    }

    #[test]
    fn paging_tokens_are_next_offsets_of_kinds_with_more() {
        let o = SearchOptions {
            from: Paging {
                messages: Some(10),
                ..Paging::default()
            },
            ..SearchOptions::default()
        };
        let r = parse_results(&response(), &o);
        // Messages: the page started at 10 and returned 3 hits (one is not usable but counts).
        assert_eq!(r.more.messages, Some(13));
        assert_eq!(r.more.files, None);
        assert_eq!(r.more.people, None);
        assert_eq!(r.more.events, Some(2));
        assert!(r.more.any());
        assert_eq!(r.total_of(SearchKind::Messages), Some(120));
        assert_eq!(r.total_of(SearchKind::Files), Some(20));
    }

    #[test]
    fn empty_and_malformed_answers_give_empty_results() {
        for v in [
            json!({}),
            json!(null),
            json!({"EntitySets": "x"}),
            json!({"EntitySets": [{}]}),
        ] {
            let r = parse_results(&v, &SearchOptions::default());
            assert_eq!(r, SearchResults::default());
        }
        assert!(!SearchResults::default().more.any());
    }

    #[test]
    fn top_results_interleave_kinds_keeping_each_kinds_order() {
        let r = parse_results(&response(), &SearchOptions::default());
        let top = r.top_results(5);
        let got: Vec<(SearchKind, usize)> = top.iter().map(|h| (h.kind, h.index)).collect();
        assert_eq!(
            got,
            [
                (SearchKind::People, 0),
                (SearchKind::Files, 0),
                (SearchKind::Messages, 0),
                (SearchKind::Events, 0),
                (SearchKind::People, 1),
            ]
        );
        assert_eq!(r.top_results(100).len(), 8);
        assert!(SearchResults::default().top_results(5).is_empty());
    }

    #[test]
    fn event_pages_are_capped() {
        let o = SearchOptions {
            page_size: 25,
            ..SearchOptions::default()
        };
        let b = query_body("x", &o, "c", "l");
        let e = b["entityRequests"].as_array().unwrap();
        assert_eq!(e[3]["entityType"], "Event");
        assert_eq!(e[3]["size"], 8);
        assert_eq!(e[0]["size"], 25);
    }

    #[test]
    fn later_pages_drop_hits_seen_earlier() {
        let first = parse_results(&response(), &SearchOptions::default());
        let mut again = first.clone();
        again.messages.push(MessageHit {
            message_id: "new".into(),
            ..first.messages[0].clone()
        });
        again.dedup_against(&first);
        assert_eq!(again.messages.len(), 1);
        assert_eq!(again.messages[0].message_id, "new");
        assert!(again.files.is_empty() && again.people.is_empty() && again.events.is_empty());
        let mut all = first.clone();
        all.append(again);
        assert_eq!(all.messages.len(), 3);
    }

    #[test]
    fn kind_names_round_trip() {
        for k in SearchKind::ALL {
            assert_eq!(SearchKind::from_name(k.name()), Some(k));
        }
        assert_eq!(SearchKind::from_name("chats"), None);
    }
}
