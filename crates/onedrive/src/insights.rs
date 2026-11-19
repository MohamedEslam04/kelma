//! Office Graph insights: the "For you" cards of Home, "Shared by you" and the People view.
//!
//! `GET /me/insights/used`, `/trending` and `/shared` answer `resourceVisualization` (title,
//! type, container) and `resourceReference` (`drives/<d>/items/<i>`) per document. Microsoft
//! marks them deprecated, so every call here degrades to an empty list (or a Graph
//! `sharedWithMe` fallback for People) instead of failing the view.

use std::collections::HashSet;

use reqwest::Method;
use serde_json::Value;
use teams_proto::time::parse_iso_ms;
use teams_proto::{Client, Error, Result};

use crate::api::{GRAPH, ItemRef, graph};
use crate::model::{DriveItem, parse_item};

/// Which insight list a document came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InsightKind {
    /// The user viewed or modified it.
    #[default]
    Used,
    /// Trending around the user.
    Trending,
    /// Shared with or by the user.
    Shared,
}

/// A person behind a share.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub address: String,
}

impl Person {
    /// The same person: by id when both have one, else by address (case-insensitive).
    pub fn same(&self, other: &Person) -> bool {
        if !self.id.is_empty() && !other.id.is_empty() {
            return self.id == other.id;
        }
        !self.address.is_empty() && self.address.eq_ignore_ascii_case(&other.address)
    }
}

/// One document from an insight list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Insight {
    pub kind: InsightKind,
    pub title: String,
    /// `Word`, `Excel`, `PowerPoint`, `Pdf`, `Visio`, `Image`, `Folder`, `Web`, `Other`, …
    pub file_type: String,
    pub media_type: String,
    pub preview_text: String,
    /// The library or person that holds it (`containerDisplayName`).
    pub container_name: String,
    /// `OneDriveBusiness`, `Site`, …
    pub container_type: String,
    pub container_url: String,
    /// Opens the document in the browser.
    pub web_url: String,
    /// The drive item, when the reference is one (`drives/<d>/items/<i>`). Use it for
    /// thumbnails (`browse::thumbnails`): `previewImageUrl` needs a SharePoint token.
    pub item: Option<ItemRef>,
    /// Last opened (used), last modified (trending) or last shared (shared).
    pub when_ms: i64,
    /// Shared insights only.
    pub shared_by: Option<Person>,
    /// `Direct`, `Link`, `Attachment`, `Group`, `Site`, … (shared insights only).
    pub sharing_type: String,
    pub sharing_subject: String,
    /// Where it was shared (for example a chat or an email), when Graph says.
    pub sharing_url: String,
}

impl Insight {
    /// A [`DriveItem`] for the file list views (name, web address, date, person); empty ids
    /// when the reference is not a drive item.
    pub fn to_drive_item(&self) -> DriveItem {
        let (drive_id, id) = self
            .item
            .as_ref()
            .map(|r| (r.drive_id.clone(), r.item_id.clone()))
            .unwrap_or_default();
        DriveItem {
            id,
            drive_id,
            name: self.title.clone(),
            is_folder: self.file_type == "Folder",
            mime: self.media_type.clone(),
            modified_ms: self.when_ms,
            modified_by: self
                .shared_by
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            web_url: self.web_url.clone(),
            ..Default::default()
        }
    }

    fn key(&self) -> String {
        match &self.item {
            Some(r) => format!("{}/{}", r.drive_id, r.item_id),
            None => self.web_url.clone(),
        }
    }
}

fn s(v: &Value, p: &str) -> String {
    v.pointer(p)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// `drives/<d>/items/<i>` (with or without a leading `/`) to an [`ItemRef`].
pub fn parse_reference_id(id: &str) -> Option<ItemRef> {
    let mut it = id.trim_start_matches('/').split('/');
    match (it.next(), it.next(), it.next(), it.next(), it.next()) {
        (Some("drives"), Some(d), Some("items"), Some(i), None)
            if !d.is_empty() && !i.is_empty() =>
        {
            Some(ItemRef::new(d, i))
        }
        _ => None,
    }
}

fn person(v: &Value) -> Option<Person> {
    let p = Person {
        id: s(v, "/id"),
        name: s(v, "/displayName"),
        address: s(v, "/address"),
    };
    (!p.id.is_empty() || !p.address.is_empty() || !p.name.is_empty()).then_some(p)
}

/// Parses one entry of `insights/used`, `trending` or `shared`. Entries without a title are
/// dropped.
pub fn parse_insight(v: &Value, kind: InsightKind) -> Option<Insight> {
    let vis = &v["resourceVisualization"];
    let title = vis["title"].as_str().filter(|t| !t.is_empty())?.to_owned();
    let when = match kind {
        InsightKind::Used => v.pointer("/lastUsed/lastAccessedDateTime"),
        InsightKind::Trending => v.pointer("/lastModifiedDateTime"),
        InsightKind::Shared => v.pointer("/lastShared/sharedDateTime"),
    }
    .and_then(Value::as_str)
    .and_then(parse_iso_ms)
    .unwrap_or(0);
    let shared = &v["lastShared"];
    Some(Insight {
        kind,
        title,
        file_type: s(vis, "/type"),
        media_type: s(vis, "/mediaType"),
        preview_text: s(vis, "/previewText"),
        container_name: s(vis, "/containerDisplayName"),
        container_type: s(vis, "/containerType"),
        container_url: s(vis, "/containerWebUrl"),
        web_url: s(v, "/resourceReference/webUrl"),
        item: v
            .pointer("/resourceReference/id")
            .and_then(Value::as_str)
            .and_then(parse_reference_id),
        when_ms: when,
        shared_by: person(&shared["sharedBy"]),
        sharing_type: s(shared, "/sharingType"),
        sharing_subject: s(shared, "/sharingSubject"),
        sharing_url: s(shared, "/sharingReference/webUrl"),
    })
}

/// Parses an insight collection.
pub fn parse_insights(v: &Value, kind: InsightKind) -> Vec<Insight> {
    v["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| parse_insight(i, kind))
        .collect()
}

/// A "For you" card: a document and why it is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct ForYouCard {
    pub insight: Insight,
    /// "You frequently open this", "Trending around you", "Ada shared this in a Teams chat".
    pub reason: String,
}

/// True when a share happened in Teams (the sharing reference is a Teams address).
fn shared_in_teams(i: &Insight) -> bool {
    url::Url::parse(&i.sharing_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|h| {
            h == "teams.microsoft.com"
                || h == "teams.cloud.microsoft"
                || h.ends_with(".teams.microsoft.com")
        })
}

/// Why a card is shown.
pub fn card_reason(i: &Insight) -> String {
    match i.kind {
        InsightKind::Used => "You frequently open this".into(),
        InsightKind::Trending => "Trending around you".into(),
        InsightKind::Shared => {
            let who = i
                .shared_by
                .as_ref()
                .map(|p| p.name.as_str())
                .filter(|n| !n.is_empty())
                .unwrap_or("Someone");
            if shared_in_teams(i) {
                format!("{who} shared this in a Teams chat")
            } else {
                format!("{who} shared this with you")
            }
        }
    }
}

/// The "For you" row: shares with the user (not by them), then used and trending documents,
/// taken in turn so each kind shows up, without duplicates and at most `max` cards. Only drive
/// items make cards: `used` also lists sites (`spsite`) and `shared` mail attachments, which
/// have no file to open or thumbnail.
pub fn for_you_cards(
    used: &[Insight],
    trending: &[Insight],
    shared: &[Insight],
    me: &Person,
    max: usize,
) -> Vec<ForYouCard> {
    let shared: Vec<&Insight> = shared
        .iter()
        .filter(|i| !i.shared_by.as_ref().is_some_and(|p| p.same(me)))
        .collect();
    let lists: [Vec<&Insight>; 3] = [shared, used.iter().collect(), trending.iter().collect()]
        .map(|l| l.into_iter().filter(|i| i.item.is_some()).collect());
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    for n in 0..longest {
        for l in &lists {
            if out.len() == max {
                return out;
            }
            if let Some(i) = l.get(n)
                && seen.insert(i.key())
            {
                out.push(ForYouCard {
                    insight: (*i).clone(),
                    reason: card_reason(i),
                });
            }
        }
    }
    out
}

/// Shares made by `me`, newest first.
pub fn shared_by(shared: &[Insight], me: &Person) -> Vec<Insight> {
    let mut v: Vec<Insight> = shared
        .iter()
        .filter(|i| i.shared_by.as_ref().is_some_and(|p| p.same(me)))
        .cloned()
        .collect();
    v.sort_by_key(|i| std::cmp::Reverse(i.when_ms));
    v
}

/// The files one person shared with the user.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonFiles {
    pub person: Person,
    pub files: Vec<DriveItem>,
    /// Newest share among `files`.
    pub latest_ms: i64,
}

/// Groups `(sharer, file)` pairs by person (not `me`), people with the newest share first and
/// each person's files newest first.
pub fn group_by_person(pairs: Vec<(Person, DriveItem)>, me: &Person) -> Vec<PersonFiles> {
    let mut out: Vec<PersonFiles> = Vec::new();
    for (p, f) in pairs {
        if p.same(me) {
            continue;
        }
        let when = f.modified_ms;
        match out.iter_mut().find(|g| g.person.same(&p)) {
            Some(g) => {
                g.latest_ms = g.latest_ms.max(when);
                g.files.push(f);
            }
            None => out.push(PersonFiles {
                person: p,
                files: vec![f],
                latest_ms: when,
            }),
        }
    }
    for g in &mut out {
        g.files.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
    }
    out.sort_by_key(|g| std::cmp::Reverse(g.latest_ms));
    out
}

/// `(sharer, file)` pairs from `/me/drive/sharedWithMe`: the sharer is
/// `remoteItem.shared.sharedBy`, else its `owner`. The file's date is the share date when
/// Graph gives one.
pub fn parse_shared_with_me_people(v: &Value) -> Vec<(Person, DriveItem)> {
    let mut out = Vec::new();
    for e in v["value"].as_array().into_iter().flatten() {
        let shared = &e["remoteItem"]["shared"];
        let who = ["sharedBy", "owner"].iter().find_map(|k| {
            let u = &shared[*k]["user"];
            let p = Person {
                id: s(u, "/id"),
                name: s(u, "/displayName"),
                address: s(u, "/email"),
            };
            (!p.name.is_empty() || !p.address.is_empty() || !p.id.is_empty()).then_some(p)
        });
        if let (Some(p), Some(mut item)) = (who, parse_item(e)) {
            if let Some(ms) = shared["sharedDateTime"].as_str().and_then(parse_iso_ms) {
                item.modified_ms = ms;
            }
            out.push((p, item));
        }
    }
    out
}

/// The signed-in user (id, name and address) for telling own shares apart.
pub fn parse_me(v: &Value) -> Person {
    Person {
        id: s(v, "/id"),
        name: s(v, "/displayName"),
        address: v["mail"]
            .as_str()
            .filter(|m| !m.is_empty())
            .or(v["userPrincipalName"].as_str())
            .unwrap_or_default()
            .to_owned(),
    }
}

/// URL of one insight list (`used` sorted by last access, newest first).
pub fn insights_url(kind: InsightKind, top: u32) -> String {
    match kind {
        InsightKind::Used => format!(
            "{GRAPH}/me/insights/used?$orderby=LastUsed/LastAccessedDateTime%20desc&$top={top}"
        ),
        InsightKind::Trending => format!("{GRAPH}/me/insights/trending?$top={top}"),
        InsightKind::Shared => format!("{GRAPH}/me/insights/shared?$top={top}"),
    }
}

async fn me(client: &Client) -> Result<Person> {
    let v = graph(
        client,
        "me",
        Method::GET,
        &format!("{GRAPH}/me?$select=id,displayName,mail,userPrincipalName"),
        None,
    )
    .await?;
    Ok(parse_me(&v))
}

/// One insight list; a deprecated or refused list is empty, not an error.
pub async fn insights(client: &Client, kind: InsightKind, top: u32) -> Vec<Insight> {
    match graph(
        client,
        "insights",
        Method::GET,
        &insights_url(kind, top),
        None,
    )
    .await
    {
        Ok(v) => parse_insights(&v, kind),
        Err(_) => Vec::new(),
    }
}

/// Home's "For you" cards (at most `max`).
pub async fn for_you(client: &Client, max: usize) -> Result<Vec<ForYouCard>> {
    let me = me(client).await?;
    let used = insights(client, InsightKind::Used, 20).await;
    let trending = insights(client, InsightKind::Trending, 20).await;
    let shared = insights(client, InsightKind::Shared, 50).await;
    Ok(for_you_cards(&used, &trending, &shared, &me, max))
}

/// "Shared by you", newest first, from the shared insights.
pub async fn shared_by_me(client: &Client) -> Result<Vec<Insight>> {
    let me = me(client).await?;
    let shared = insights(client, InsightKind::Shared, 200).await;
    Ok(shared_by(&shared, &me))
}

/// "Browse files by People": who shared files with the user. The shared insights first; when
/// they give nobody, the sharers recorded in `/me/drive/sharedWithMe`.
pub async fn people(client: &Client) -> Result<Vec<PersonFiles>> {
    let me = me(client).await?;
    let pairs: Vec<(Person, DriveItem)> = insights(client, InsightKind::Shared, 200)
        .await
        .into_iter()
        .filter_map(|i| {
            let f = i.to_drive_item();
            i.shared_by.map(|p| (p, f))
        })
        .collect();
    let grouped = group_by_person(pairs, &me);
    if !grouped.is_empty() {
        return Ok(grouped);
    }
    let v = graph(
        client,
        "shared_with_me",
        Method::GET,
        &format!("{GRAPH}/me/drive/sharedWithMe"),
        None,
    )
    .await
    .map_err(|e| Error::Protocol(format!("people: {e}")))?;
    Ok(group_by_person(parse_shared_with_me_people(&v), &me))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shared(title: &str, by: &str, id: &str, when: &str, sharing_url: &str) -> Value {
        json!({
            "id": format!("ins-{title}"),
            "lastShared": {
                "sharedDateTime": when,
                "sharingType": "Direct",
                "sharedBy": {"displayName": by, "address": format!("{}@contoso.com", by.to_lowercase()), "id": id},
                "sharingReference": {"webUrl": sharing_url}
            },
            "resourceVisualization": {
                "title": title, "type": "Word",
                "mediaType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "previewImageUrl": "https://contoso-my.sharepoint.com/_api/v2.0/drives/b!x/items/01A/thumbnails/0/small/thumbnailContent",
                "previewText": "Quarterly numbers",
                "containerWebUrl": "https://contoso-my.sharepoint.com/personal/ada/Documents",
                "containerDisplayName": by, "containerType": "OneDriveBusiness"
            },
            "resourceReference": {
                "webUrl": format!("https://contoso-my.sharepoint.com/{title}"),
                "id": format!("drives/b!x/items/01{title}"),
                "type": "microsoft.graph.driveItem"
            }
        })
    }

    fn me() -> Person {
        Person {
            id: "u-me".into(),
            name: "Me".into(),
            address: "me@contoso.com".into(),
        }
    }

    #[test]
    fn parses_used_trending_and_shared_insights() {
        let used = parse_insights(
            &json!({"value": [{
                "id": "a",
                "lastUsed": {"lastAccessedDateTime": "2026-11-10T08:30:00Z", "lastModifiedDateTime": "2026-11-01T00:00:00Z"},
                "resourceVisualization": {"title": "Org chart", "type": "Visio", "mediaType": "application/vnd.visio",
                    "containerDisplayName": "Retail", "containerType": "Site"},
                "resourceReference": {"webUrl": "https://contoso.sharepoint.com/x", "id": "drives/b!d/items/01V", "type": "microsoft.graph.driveItem"}
            }, {"id": "no title", "resourceVisualization": {}}]}),
            InsightKind::Used,
        );
        assert_eq!(used.len(), 1);
        let u = &used[0];
        assert_eq!(
            (u.title.as_str(), u.file_type.as_str()),
            ("Org chart", "Visio")
        );
        assert_eq!(u.item, Some(ItemRef::new("b!d", "01V")));
        assert_eq!(u.when_ms, 1_794_299_400_000);
        assert_eq!(u.container_type, "Site");
        assert!(u.shared_by.is_none());

        let t = parse_insight(
            &json!({"id": "t", "weight": 0.5, "lastModifiedDateTime": "2026-11-10T08:30:00Z",
                "resourceVisualization": {"title": "Deck", "type": "PowerPoint"},
                "resourceReference": {"webUrl": "https://c/x", "id": "sites/s/lists/l/items/1"}}),
            InsightKind::Trending,
        )
        .unwrap();
        assert_eq!(t.when_ms, 1_794_299_400_000);
        assert_eq!(t.item, None, "only drive items get an ItemRef");

        let s = parse_insight(
            &shared("Plan", "Ada", "u-ada", "2026-11-10T08:30:00Z", ""),
            InsightKind::Shared,
        )
        .unwrap();
        let by = s.shared_by.clone().unwrap();
        assert_eq!(
            (by.name.as_str(), by.address.as_str()),
            ("Ada", "ada@contoso.com")
        );
        assert_eq!(s.sharing_type, "Direct");
        assert_eq!(s.when_ms, 1_794_299_400_000);
        let d = s.to_drive_item();
        assert_eq!(
            (d.drive_id.as_str(), d.id.as_str(), d.name.as_str()),
            ("b!x", "01Plan", "Plan")
        );
        assert_eq!(d.modified_by, "Ada");
        assert!(d.mime.contains("wordprocessingml"));
    }

    #[test]
    fn reference_ids() {
        assert_eq!(
            parse_reference_id("drives/b!d/items/01A"),
            Some(ItemRef::new("b!d", "01A"))
        );
        assert_eq!(
            parse_reference_id("/drives/b!d/items/01A"),
            Some(ItemRef::new("b!d", "01A"))
        );
        assert_eq!(parse_reference_id("drives/b!d/items/01A/children"), None);
        assert_eq!(parse_reference_id("sites/x/lists/y/items/1"), None);
        assert_eq!(parse_reference_id("drives//items/01A"), None);
    }

    #[test]
    fn card_reasons() {
        let teams = parse_insight(
            &shared(
                "A",
                "Ada",
                "u-ada",
                "2026-11-10T08:30:00Z",
                "https://teams.microsoft.com/l/message/19:x",
            ),
            InsightKind::Shared,
        )
        .unwrap();
        assert_eq!(card_reason(&teams), "Ada shared this in a Teams chat");
        let mail = parse_insight(
            &shared(
                "B",
                "Bob",
                "u-bob",
                "2026-11-10T08:30:00Z",
                "https://outlook.office.com/mail/x",
            ),
            InsightKind::Shared,
        )
        .unwrap();
        assert_eq!(card_reason(&mail), "Bob shared this with you");
        let spoof = parse_insight(
            &shared(
                "C",
                "Eve",
                "u-eve",
                "2026-11-10T08:30:00Z",
                "https://teams.microsoft.com.evil.test/x",
            ),
            InsightKind::Shared,
        )
        .unwrap();
        assert_eq!(card_reason(&spoof), "Eve shared this with you");
        let used = Insight {
            kind: InsightKind::Used,
            ..Default::default()
        };
        assert_eq!(card_reason(&used), "You frequently open this");
        let trending = Insight {
            kind: InsightKind::Trending,
            ..Default::default()
        };
        assert_eq!(card_reason(&trending), "Trending around you");
    }

    fn ins(kind: InsightKind, title: &str) -> Insight {
        Insight {
            kind,
            title: title.into(),
            item: Some(ItemRef::new("d", title)),
            ..Default::default()
        }
    }

    #[test]
    fn for_you_takes_kinds_in_turn_without_duplicates_or_own_shares() {
        let mut mine = ins(InsightKind::Shared, "Mine");
        mine.shared_by = Some(me());
        let mut theirs = ins(InsightKind::Shared, "Same");
        theirs.shared_by = Some(Person {
            id: "u-ada".into(),
            name: "Ada".into(),
            ..Default::default()
        });
        let used = [
            ins(InsightKind::Used, "Same"),
            ins(InsightKind::Used, "U2"),
            ins(InsightKind::Used, "U3"),
        ];
        let trending = [ins(InsightKind::Trending, "T1")];
        let site = Insight {
            kind: InsightKind::Used,
            title: "A site".into(),
            ..Default::default()
        };
        let used = [used[0].clone(), site, used[1].clone(), used[2].clone()];
        let cards = for_you_cards(&used, &trending, &[mine, theirs], &me(), 10);
        let titles: Vec<&str> = cards.iter().map(|c| c.insight.title.as_str()).collect();
        assert_eq!(titles, ["Same", "T1", "U2", "U3"]);
        assert_eq!(cards[0].reason, "Ada shared this with you");
        assert_eq!(for_you_cards(&used, &trending, &[], &me(), 2).len(), 2);
        assert!(for_you_cards(&[], &[], &[], &me(), 5).is_empty());
    }

    #[test]
    fn shared_by_me_keeps_own_shares_newest_first() {
        let v = json!({"value": [
            shared("Old", "Me", "u-me", "2026-11-01T00:00:00Z", ""),
            shared("Theirs", "Ada", "u-ada", "2026-11-05T00:00:00Z", ""),
            shared("New", "Me", "u-me", "2026-11-09T00:00:00Z", ""),
        ]});
        let all = parse_insights(&v, InsightKind::Shared);
        let mine: Vec<String> = shared_by(&all, &me())
            .into_iter()
            .map(|i| i.title)
            .collect();
        assert_eq!(mine, ["New", "Old"]);
        // Matched by address when an id is missing.
        let by_addr = Person {
            address: "ME@contoso.com".into(),
            ..Default::default()
        };
        assert_eq!(shared_by(&all, &by_addr).len(), 2);
    }

    #[test]
    fn people_groups_sharers_newest_first() {
        let v = json!({"value": [
            shared("A1", "Ada", "u-ada", "2026-11-01T00:00:00Z", ""),
            shared("B1", "Bob", "u-bob", "2026-11-03T00:00:00Z", ""),
            shared("A2", "Ada", "u-ada", "2026-11-05T00:00:00Z", ""),
            shared("M1", "Me", "u-me", "2026-11-09T00:00:00Z", ""),
        ]});
        let pairs = parse_insights(&v, InsightKind::Shared)
            .into_iter()
            .filter_map(|i| {
                let f = i.to_drive_item();
                i.shared_by.map(|p| (p, f))
            })
            .collect();
        let g = group_by_person(pairs, &me());
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].person.name, "Ada");
        let names: Vec<&str> = g[0].files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["A2", "A1"]);
        assert_eq!(g[1].person.name, "Bob");
    }

    #[test]
    fn people_from_shared_with_me() {
        let v = json!({"value": [
            {"id": "s1", "name": "Plan.docx", "parentReference": {"driveId": "mine"},
             "remoteItem": {"id": "01T", "parentReference": {"driveId": "b!t"},
                "shared": {"sharedDateTime": "2026-11-10T08:30:00Z",
                    "sharedBy": {"user": {"id": "u-ada", "displayName": "Ada", "email": "ada@contoso.com"}},
                    "owner": {"user": {"id": "u-own", "displayName": "Owner"}}}}},
            {"id": "s2", "name": "Notes.txt", "parentReference": {"driveId": "mine"},
             "remoteItem": {"id": "01U", "parentReference": {"driveId": "b!t"},
                "shared": {"owner": {"user": {"id": "u-bob", "displayName": "Bob"}}}}},
            {"id": "s3", "name": "Nobody.txt", "remoteItem": {"id": "01W"}}
        ]});
        let p = parse_shared_with_me_people(&v);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].0.name, "Ada");
        assert_eq!(p[0].1.id, "01T");
        assert_eq!(p[0].1.modified_ms, 1_794_299_400_000);
        assert_eq!(p[1].0.name, "Bob");
    }

    #[test]
    fn me_and_urls() {
        let m = parse_me(
            &json!({"id": "u1", "displayName": "Me", "mail": null, "userPrincipalName": "me@contoso.com"}),
        );
        assert_eq!(m.address, "me@contoso.com");
        assert_eq!(
            insights_url(InsightKind::Used, 20),
            "https://graph.microsoft.com/v1.0/me/insights/used?$orderby=LastUsed/LastAccessedDateTime%20desc&$top=20"
        );
        assert!(insights_url(InsightKind::Shared, 5).ends_with("/me/insights/shared?$top=5"));
        assert!(insights_url(InsightKind::Trending, 5).ends_with("/me/insights/trending?$top=5"));
    }
}
