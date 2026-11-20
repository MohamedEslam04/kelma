//! Global search (plan X3.4): the Files, People and Meetings tabs of the Search page come from one
//! substrate query (`teams_proto::global_search`), with a page per tab. When that query fails the
//! older per-kind calls (directory people, OneDrive files) answer instead.

use super::{Hit, PAGE, State, Tab};
use crate::backend::Engine;
use chrono::Datelike;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use teams_proto::error::Result;
use teams_proto::global_search::{
    EventHit, FileHit, Paging, PersonHit, SearchKind, SearchOptions, SearchResults,
};

/// Hits of each kind asked for by the first query of a search.
const FIRST_PAGE: u32 = 10;

/// The kinds the first query asks for (messages come from their own, older path).
const OTHER_KINDS: [SearchKind; 3] = [SearchKind::Files, SearchKind::People, SearchKind::Events];

/// The global-search kind a category tab pages through.
pub(super) fn kind_of_tab(tab: Tab) -> Option<SearchKind> {
    match tab {
        Tab::Files => Some(SearchKind::Files),
        Tab::People => Some(SearchKind::People),
        Tab::Meetings => Some(SearchKind::Events),
        _ => None,
    }
}

fn set_next(p: &mut Paging, kind: SearchKind, next: Option<u32>) {
    match kind {
        SearchKind::Messages => p.messages = next,
        SearchKind::Files => p.files = next,
        SearchKind::People => p.people = next,
        SearchKind::Events => p.events = next,
    }
}

fn file_hit(f: &FileHit) -> Hit {
    let folder = f
        .parent_url
        .as_deref()
        .and_then(|u| u.trim_end_matches('/').rsplit('/').next())
        .map(|s| s.replace("%20", " "));
    Hit {
        kind: "file",
        // Not read for files; makes two files without a URL differ.
        message: f.item_id.clone().unwrap_or_else(|| f.name.clone()),
        name: f.name.clone(),
        place: [f.author.clone(), folder]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
        time_ms: f.modified_ms.unwrap_or(0),
        url: f.url.clone().unwrap_or_default(),
        ..Default::default()
    }
}

fn person_hit(p: &PersonHit) -> Hit {
    Hit {
        kind: "person",
        mri: p.mri.clone(),
        name: p.name.clone(),
        place: [p.title.clone(), p.email.clone()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
        ..Default::default()
    }
}

/// A calendar event. It has no chat (`conv` is empty), which tells it from a meeting chat.
fn event_hit(e: &EventHit) -> Hit {
    Hit {
        kind: "meeting",
        message: e.id.clone(),
        name: if e.cancelled {
            format!("Canceled: {}", e.subject)
        } else {
            e.subject.clone()
        },
        place: e.organizer.clone().unwrap_or_default(),
        time_ms: e.start_ms.unwrap_or(0),
        ..Default::default()
    }
}

/// Files, people and events of an answer as rows, in the service's order.
pub(super) fn hits_of(r: &SearchResults) -> Vec<Hit> {
    r.files
        .iter()
        .map(file_hit)
        .chain(r.people.iter().map(person_hit))
        .chain(r.events.iter().map(event_hit))
        .collect()
}

/// True for the rows a global answer replaces: files, people and calendar events (a meeting chat
/// from the cache has a conversation and stays).
pub(super) fn is_global(h: &Hit) -> bool {
    matches!(h.kind, "file" | "person") || (h.kind == "meeting" && h.conv.is_empty())
}

/// Adds the hits that are not in the list yet (a later page repeats the service's top results).
pub(super) fn add_new(hits: &mut Vec<Hit>, new: Vec<Hit>) {
    for n in new {
        let id = n.id();
        if !hits.iter().any(|h| h.id() == id) {
            hits.push(n);
        }
    }
}

/// Whether the tab has another page to load.
pub(super) fn tab_has_more(tab: Tab, more: &Paging) -> bool {
    kind_of_tab(tab).is_some_and(|k| more.get(k).is_some())
}

/// What the first query of a search found besides messages.
pub(super) struct Others {
    pub hits: Vec<Hit>,
    pub more: Paging,
}

impl Engine {
    /// Files, people and events for `q`: one global search, and the per-kind calls when it fails.
    pub(super) async fn others_first(self: &Arc<Self>, q: &str) -> Others {
        let opts = SearchOptions {
            kinds: OTHER_KINDS.to_vec(),
            page_size: FIRST_PAGE,
            from: Paging::default(),
        };
        match self.client.global_search(&self.me, q, &opts).await {
            Ok(r) => Others {
                hits: hits_of(&r),
                more: r.more,
            },
            Err(err) => {
                tracing::warn!(%err, "global search failed, using the per-kind calls");
                Others {
                    hits: self.legacy_others(q).await,
                    more: Paging::default(),
                }
            }
        }
    }

    /// The directory and OneDrive searches of before the global search.
    async fn legacy_others(self: &Arc<Self>, q: &str) -> Vec<Hit> {
        let (people, files) = tokio::join!(
            async { self.client.search_people(&self.me, q).await.ok() },
            async { onedrive::search(&self.client, q).await.ok() }
        );
        let mut hits = vec![];
        hits.extend(people.into_iter().flatten().take(10).map(|p| {
            Hit {
                kind: "person",
                mri: p.mri,
                name: p.display_name,
                place: [p.job_title, p.email]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
                ..Default::default()
            }
        }));
        hits.extend(
            files
                .into_iter()
                .flatten()
                .filter(|i| !i.is_folder)
                .map(|i| Hit {
                    kind: "file",
                    name: i.name,
                    place: [i.modified_by, i.parent_path]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · "),
                    time_ms: i.modified_ms,
                    url: i.web_url,
                    ..Default::default()
                }),
        );
        hits
    }

    /// "Show more" on the Files, People or Meetings tab: the next page of that kind.
    pub(super) async fn global_more(self: &Arc<Self>, kind: SearchKind) {
        let (q, seq, from) = {
            let f = self.find.lock().unwrap();
            (f.query.clone(), f.seq, f.global_more)
        };
        if from.get(kind).is_none() || self.offline.load(Ordering::Relaxed) {
            return;
        }
        self.push_search(true);
        let opts = SearchOptions {
            kinds: vec![kind],
            page_size: PAGE,
            from,
        };
        let page = self.client.global_search(&self.me, &q, &opts).await;
        self.apply_page(seq, kind, page);
    }

    /// Adds a page of `kind` to the results of search number `seq`.
    fn apply_page(self: &Arc<Self>, seq: u64, kind: SearchKind, page: Result<SearchResults>) {
        {
            let mut f = self.find.lock().unwrap();
            if f.seq != seq {
                return;
            }
            match page {
                Ok(r) => {
                    add_new(&mut f.hits, hits_of(&r));
                    set_next(&mut f.global_more, kind, r.more.get(kind));
                }
                Err(err) => {
                    set_next(&mut f.global_more, kind, None);
                    tracing::warn!(%err, "more search results failed");
                }
            }
        }
        self.push_search(false);
    }

    /// Tests: the first answer of the current search.
    #[cfg(test)]
    pub(crate) fn test_global_first(self: &Arc<Self>, r: SearchResults) {
        store_others(
            &mut self.find.lock().unwrap(),
            Others {
                hits: hits_of(&r),
                more: r.more,
            },
        );
        self.push_search(false);
    }

    /// Tests: a page that answers "Show more" on the tab of `kind`.
    #[cfg(test)]
    pub(crate) fn test_global_page(self: &Arc<Self>, kind: SearchKind, r: SearchResults) {
        let seq = self.find.lock().unwrap().seq;
        self.apply_page(seq, kind, Ok(r));
    }

    /// An event row was opened: the calendar on the day of the event.
    pub(super) async fn open_event(self: &Arc<Self>, start_ms: i64) {
        self.ui.show_view("calendar");
        if let Some(t) = appearance::formats::local(start_ms).filter(|_| start_ms > 0) {
            let date = calendar::Date {
                year: t.year(),
                month: t.month(),
                day: t.day(),
            };
            self.clone()
                .handle(crate::backend::Cmd::Calendar(crate::backend::CalCmd::Goto(
                    date,
                )))
                .await;
        }
    }
}

/// Stores a first answer in the search state.
pub(super) fn store_others(f: &mut State, o: Others) {
    f.hits.retain(|h| !is_global(h));
    f.hits.extend(o.hits);
    f.global_more = o.more;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, url: Option<&str>) -> FileHit {
        FileHit {
            name: name.into(),
            url: url.map(str::to_owned),
            parent_url: Some("https://contoso.example/sites/x/Shared%20Documents/".into()),
            site_id: None,
            web_id: None,
            list_id: None,
            item_id: None,
            extension: Some("docx".into()),
            size: None,
            modified_ms: Some(1_700_000_000_000),
            author: Some("Sara".into()),
        }
    }

    fn results() -> SearchResults {
        SearchResults {
            files: vec![
                file("plan.docx", Some("https://c/plan.docx")),
                file("b.docx", None),
            ],
            people: vec![PersonHit {
                mri: "8:orgid:p1".into(),
                name: "Omar".into(),
                email: Some("omar@c.example".into()),
                title: Some("Engineer".into()),
            }],
            events: vec![EventHit {
                id: "AAMk1".into(),
                subject: "Sprint review".into(),
                start_ms: Some(1_700_000_000_000),
                end_ms: None,
                organizer: Some("Lina".into()),
                cancelled: true,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn answers_become_rows_of_each_category() {
        let hits = hits_of(&results());
        let kinds: Vec<_> = hits.iter().map(|h| h.kind).collect();
        assert_eq!(kinds, ["file", "file", "person", "meeting"]);
        assert_eq!(hits[0].url, "https://c/plan.docx");
        assert_eq!(hits[0].place, "Sara · Shared Documents");
        assert_eq!(hits[2].place, "Engineer · omar@c.example");
        assert_eq!(hits[3].name, "Canceled: Sprint review");
        assert_eq!(hits[3].place, "Lina");
        assert!(hits[3].conv.is_empty(), "an event has no chat");
        assert_ne!(
            hits[0].id(),
            hits[1].id(),
            "files without a URL still differ"
        );
    }

    #[test]
    fn tabs_map_to_kinds_and_pages() {
        assert_eq!(kind_of_tab(Tab::Files), Some(SearchKind::Files));
        assert_eq!(kind_of_tab(Tab::People), Some(SearchKind::People));
        assert_eq!(kind_of_tab(Tab::Meetings), Some(SearchKind::Events));
        assert_eq!(kind_of_tab(Tab::Messages), None);
        assert_eq!(kind_of_tab(Tab::All), None);
        let more = Paging {
            files: Some(10),
            ..Default::default()
        };
        assert!(tab_has_more(Tab::Files, &more));
        assert!(!tab_has_more(Tab::People, &more));
        assert!(
            !tab_has_more(Tab::All, &more),
            "the All tab pages messages only"
        );
    }

    #[test]
    fn a_later_page_adds_only_new_hits() {
        let mut hits = hits_of(&results());
        let mut again = hits_of(&results());
        again.push(file_hit(&file("c.docx", Some("https://c/c.docx"))));
        add_new(&mut hits, again);
        assert_eq!(hits.len(), 5);
        assert_eq!(hits[4].name, "c.docx");
    }

    #[test]
    fn a_new_answer_replaces_global_rows_and_keeps_the_cache() {
        let mut f = State::default();
        f.hits.push(Hit {
            kind: "meeting",
            conv: "19:meet@thread.v2".into(),
            name: "Standup".into(),
            ..Default::default()
        });
        f.hits.push(Hit {
            kind: "message",
            ..Default::default()
        });
        f.hits.push(Hit {
            kind: "file",
            name: "old.docx".into(),
            ..Default::default()
        });
        store_others(
            &mut f,
            Others {
                hits: hits_of(&results()),
                more: Paging {
                    files: Some(10),
                    ..Default::default()
                },
            },
        );
        let names: Vec<_> = f.hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Standup",
                "",
                "plan.docx",
                "b.docx",
                "Omar",
                "Canceled: Sprint review"
            ]
        );
        assert_eq!(f.global_more.files, Some(10));
    }
}
