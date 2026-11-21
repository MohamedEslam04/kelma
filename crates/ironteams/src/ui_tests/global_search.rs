//! Global search (plan X3.4) driven through the real window: the Files, People and Meetings tabs
//! of the Search page show what one global query found, and "Show more" pages that kind. The
//! engine has no network, so the query's answers are handed to it by hand.

use super::*;
use crate::backend::FindCmd;
use crate::{FindStore, HitRow};
use teams_proto::global_search::{EventHit, FileHit, Paging, PersonHit, SearchKind, SearchResults};

fn wire(h: &Harness) -> Log {
    let log = Log::default();
    let run = h.runner();
    let l = log.clone();
    h.app.global::<FindStore>().on_command(move |name, arg| {
        l.borrow_mut().push(format!("{name} {arg}"));
        if let Some(c) = FindCmd::parse(&name, &arg) {
            run(Cmd::Find(c));
        }
    });
    log
}

fn rows(app: &AppWindow) -> Vec<HitRow> {
    app.global::<FindStore>().get_results().iter().collect()
}

fn file(name: &str) -> FileHit {
    FileHit {
        name: name.into(),
        url: Some(format!("https://files.invalid/{name}")),
        parent_url: None,
        site_id: None,
        web_id: None,
        list_id: None,
        item_id: None,
        extension: Some("docx".into()),
        size: None,
        modified_ms: Some(1_795_000_000_000),
        author: Some("Sara Hassan".into()),
    }
}

fn files(names: &[&str], next: Option<u32>) -> SearchResults {
    SearchResults {
        files: names.iter().map(|n| file(n)).collect(),
        more: Paging {
            files: next,
            ..Paging::default()
        },
        ..SearchResults::default()
    }
}

/// A search for a phrase the demo cache holds, so the page is in its "searched" state.
fn search(h: &Harness) -> Log {
    let log = wire(h);
    h.runner()(Cmd::Find(FindCmd::Search("deployed to".into())));
    log
}

fn first(h: &Harness, r: SearchResults) {
    h.rt.block_on(async { h.engine.test_global_first(r) });
}

fn page(h: &Harness, kind: SearchKind, r: SearchResults) {
    h.rt.block_on(async { h.engine.test_global_page(kind, r) });
}

fn open_tab(h: &Harness, label: &str) {
    let tab = find_label(&h.app, label)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no tab {label}"));
    tab.mock_single_click(PointerEventButton::Left);
}

#[test]
fn the_files_tab_lists_global_hits_and_pages_with_show_more() {
    let h = Harness::new();
    let log = search(&h);
    first(
        &h,
        files(&["q3-plan.docx", "budget.xlsx", "notes.docx"], Some(3)),
    );

    open_tab(&h, "Files");
    let r = rows(&h.app);
    assert_eq!(r.len(), 3);
    assert!(r.iter().all(|r| r.kind == "file"));
    assert_eq!(r[0].title, "q3-plan.docx");
    assert_eq!(r[0].subtitle, "Sara Hassan");
    assert!(h.app.global::<FindStore>().get_more(), "a next page exists");

    open_tab(&h, "Show more results");
    assert_eq!(log.borrow().last().unwrap(), "more ");
    // The answer of that page repeats one hit and adds two.
    page(
        &h,
        SearchKind::Files,
        files(&["notes.docx", "spec.pdf", "old.txt"], None),
    );
    let r = rows(&h.app);
    let titles: Vec<_> = r.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "q3-plan.docx",
            "budget.xlsx",
            "notes.docx",
            "spec.pdf",
            "old.txt"
        ],
        "the repeated hit is not listed twice"
    );
    assert!(!h.app.global::<FindStore>().get_more(), "no further page");
}

#[test]
fn the_people_tab_lists_directory_hits() {
    let h = Harness::new();
    search(&h);
    first(
        &h,
        SearchResults {
            people: vec![PersonHit {
                mri: "8:orgid:00000000-0000-0000-0000-0000000000aa".into(),
                name: "Omar Khaled".into(),
                email: Some("omar@contoso.invalid".into()),
                title: Some("Engineer".into()),
            }],
            ..SearchResults::default()
        },
    );

    open_tab(&h, "People");
    let r = rows(&h.app);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].kind, "person");
    assert_eq!(r[0].title, "Omar Khaled");
    assert_eq!(r[0].subtitle, "Engineer · omar@contoso.invalid");
    assert!(!h.app.global::<FindStore>().get_more());
}

#[test]
fn the_meetings_tab_lists_events_and_opening_one_shows_the_calendar() {
    let h = Harness::new();
    let log = search(&h);
    first(
        &h,
        SearchResults {
            events: vec![EventHit {
                id: "AAMkEvent1".into(),
                subject: "Sprint review".into(),
                start_ms: Some(1_795_000_000_000),
                end_ms: Some(1_795_003_600_000),
                organizer: Some("Lina Samir".into()),
                cancelled: false,
            }],
            more: Paging {
                events: Some(1),
                ..Paging::default()
            },
            ..SearchResults::default()
        },
    );

    open_tab(&h, "Meetings");
    let r = rows(&h.app);
    let event = r
        .iter()
        .find(|r| r.title == "Sprint review")
        .expect("the event is listed");
    assert_eq!(event.kind, "meeting");
    assert_eq!(event.subtitle, "Lina Samir");
    assert!(h.app.global::<FindStore>().get_more(), "events page too");

    h.app
        .global::<FindStore>()
        .invoke_command("open".into(), event.id.clone());
    assert_eq!(log.borrow().last().unwrap(), &format!("open {}", event.id));
    assert_eq!(h.app.get_view(), "calendar", "an event opens the calendar");
}

#[test]
fn the_all_tab_shows_the_top_of_each_global_category() {
    let h = Harness::new();
    search(&h);
    first(
        &h,
        SearchResults {
            files: ["a", "b", "c", "d", "e"].iter().map(|n| file(n)).collect(),
            people: vec![PersonHit {
                mri: "8:orgid:p1".into(),
                name: "Omar".into(),
                email: None,
                title: None,
            }],
            ..SearchResults::default()
        },
    );
    let r = rows(&h.app);
    let files: Vec<_> = r.iter().filter(|r| r.kind == "file").collect();
    assert_eq!(files.len(), 3, "the All tab caps a category at three");
    assert_eq!(files[0].section, "Files");
    let people = r.iter().find(|r| r.kind == "person").unwrap();
    assert_eq!(people.section, "People");
}
