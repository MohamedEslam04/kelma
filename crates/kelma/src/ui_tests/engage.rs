//! The Viva Engage feed (plan S3.2) driven through the real window. The real engine runs behind
//! it with a scripted feed (`engage::Fake`), so no request leaves the process: every
//! `EngageStore.command` goes through `EngageCmd::parse` and the engine, and what the window
//! shows is read back from the rendered elements. The feed is read only: the tests also check
//! that the post, reply and like controls cannot be used.

use super::*;
use crate::backend::EngageCmd;
use crate::backend::engage::{Fake, fake_post};
use crate::{EngageStore, TCommunity, TeamsStore};
use i_slint_backend_testing::AccessibleRole;
use teams_proto::engage::{Community, EngagePost};

const COMMUNITY: u64 = 42;
const NAME: &str = "All Company";

fn es(app: &AppWindow) -> EngageStore<'_> {
    app.global::<EngageStore>()
}

fn community() -> Community {
    Community {
        id: COMMUNITY,
        name: NAME.into(),
        description: String::new(),
        privacy: "public".into(),
        members: 300,
        is_company_wide: true,
        last_message_ms: 0,
        web_url: String::new(),
    }
}

fn post(id: u64, sender: &str, text: &str) -> EngagePost {
    let mut p = fake_post(id, id, None, text);
    p.sender_name = sender.into();
    p
}

/// `n` first posts with the ids `from + n - 1` down to `from`, as a feed page is.
fn page(from: u64, n: u64) -> Vec<EngagePost> {
    (from..from + n)
        .rev()
        .map(|i| post(i, "Pat Poster", &format!("post number {i}")))
        .collect()
}

struct EngageHarness {
    h: Harness,
    /// Every `name argument` the view sent, in order.
    log: Log,
}

impl EngageHarness {
    /// A window whose chat list has the community, with the feed scripted as `pages`.
    fn new(pages: Vec<Result<Vec<EngagePost>, String>>) -> Self {
        let h = Harness::new();
        h.engine.engage_script(Fake {
            communities: vec![community()],
            pages: pages.into(),
            requests: vec![],
        });
        let log = Log::default();
        let (l, run) = (log.clone(), h.runner());
        es(&h.app).on_command(move |name, arg| {
            l.borrow_mut()
                .push(format!("{name} {arg}").trim().to_owned());
            if let Some(c) = EngageCmd::parse(&name, &arg) {
                run(Cmd::Engage(c));
            }
        });
        h.app
            .global::<TeamsStore>()
            .set_communities(ModelRc::new(VecModel::from(vec![TCommunity {
                id: COMMUNITY.to_string().into(),
                name: NAME.into(),
                people: 300,
                company: true,
                ..Default::default()
            }])));
        EngageHarness { h, log }
    }

    /// Clicks the community in the chat list, as a user opens it.
    fn open(&self) {
        let row = list_items(&self.h.app, NAME).remove(0);
        click(&row);
    }

    fn app(&self) -> &AppWindow {
        &self.h.app
    }
}

/// A window tall enough for a whole page of posts, so the pager is inside the visible area.
fn tall(e: &EngageHarness) {
    e.app().window().set_size(LogicalSize::new(1200., 6000.));
}

fn with_role(app: &AppWindow, label: &str, role: AccessibleRole) -> Vec<ElementHandle> {
    ElementHandle::find_by_accessible_label(app, label)
        .filter(|e| e.accessible_role() == Some(role.clone()))
        .collect()
}

fn list_items(app: &AppWindow, label: &str) -> Vec<ElementHandle> {
    with_role(app, label, AccessibleRole::ListItem)
}

fn buttons(app: &AppWindow, label: &str) -> Vec<ElementHandle> {
    with_role(app, label, AccessibleRole::Button)
}

fn has_text(app: &AppWindow, label: &str) -> bool {
    !ElementHandle::find_by_accessible_label(app, label)
        .collect::<Vec<_>>()
        .is_empty()
}

fn posts_shown(app: &AppWindow) -> Vec<String> {
    es(app)
        .get_posts()
        .iter()
        .map(|p| p.id.to_string())
        .collect()
}

#[test]
fn opening_a_community_shows_its_posts_from_fake_data() {
    let mut first = post(10, "Sara Example", "Welcome to the community");
    first.likes = 3;
    first.attachments = 1;
    let mut rich = post(8, "Omar Example", "");
    rich.rich = "<p>Rich <b>body</b></p>".into();
    let reply_a = fake_post(12, 10, Some(10), "first reply");
    let reply_b = fake_post(11, 10, Some(10), "second reply");
    let e = EngageHarness::new(vec![Ok(vec![reply_a, reply_b, first, rich])]);
    assert!(!es(e.app()).get_open());
    e.open();
    assert_eq!(*e.log.borrow(), vec![format!("open {COMMUNITY}")]);
    assert!(es(e.app()).get_open());
    assert_eq!(
        es(e.app()).get_title(),
        NAME,
        "the title comes from the community list"
    );
    assert_eq!(
        e.h.engine.engage_requests(),
        vec![(COMMUNITY, None)],
        "one read of the newest page"
    );
    // Replies are counted on their post, not listed; the two posts are.
    assert_eq!(posts_shown(e.app()), ["10", "8"]);
    let sara = list_items(e.app(), "Sara Example");
    assert_eq!(sara.len(), 1);
    assert_eq!(
        sara[0].accessible_description().unwrap(),
        "Welcome to the community"
    );
    assert!(has_text(e.app(), "3 likes"));
    assert!(has_text(e.app(), "2 replies"));
    assert!(has_text(e.app(), "1 attachment"));
    assert!(has_text(e.app(), "0 replies"));
    let omar = list_items(e.app(), "Omar Example");
    assert!(
        omar[0]
            .accessible_description()
            .unwrap()
            .contains("Rich body"),
        "a post with only a rich body shows its text"
    );
    assert!(!es(e.app()).get_loading());
    assert!(!has_text(e.app(), "No posts yet"));
}

#[test]
fn the_older_pager_loads_the_next_page_below() {
    let e = EngageHarness::new(vec![Ok(page(100, 20)), Ok(page(90, 10))]);
    tall(&e);
    e.open();
    assert_eq!(es(e.app()).get_posts().row_count(), 20);
    assert!(es(e.app()).get_more());
    let older = buttons(e.app(), "Older posts");
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].accessible_enabled(), Some(true));
    click(&older[0]);
    assert_eq!(e.log.borrow().last().unwrap(), "older");
    assert_eq!(
        e.h.engine.engage_requests(),
        vec![(COMMUNITY, None), (COMMUNITY, Some(100))],
        "the older page is asked for from the oldest post shown"
    );
    let ids = posts_shown(e.app());
    assert_eq!(ids.len(), 30);
    assert_eq!(ids.first().unwrap(), "119");
    assert_eq!(ids.last().unwrap(), "90", "older posts are added below");
    assert!(!es(e.app()).get_more(), "a short page is the last one");
    assert!(buttons(e.app(), "Older posts").is_empty());
}

#[test]
fn the_pager_waits_while_a_page_is_loading() {
    let e = EngageHarness::new(vec![Ok(page(100, 20))]);
    tall(&e);
    e.open();
    es(e.app()).set_loading(true);
    let older = buttons(e.app(), "Older posts");
    assert_eq!(older[0].accessible_enabled(), Some(false));
    assert!(has_text(e.app(), "Loading"));
}

#[test]
fn the_refresh_button_reloads_the_newest_page() {
    let e = EngageHarness::new(vec![Ok(page(10, 2)), Ok(page(20, 3))]);
    e.open();
    assert_eq!(posts_shown(e.app()), ["11", "10"]);
    click(&buttons(e.app(), "Refresh")[0]);
    assert_eq!(e.log.borrow().last().unwrap(), "refresh");
    assert_eq!(posts_shown(e.app()), ["22", "21", "20"]);
    assert_eq!(
        e.h.engine.engage_requests(),
        vec![(COMMUNITY, None), (COMMUNITY, None)]
    );
}

#[test]
fn a_failed_load_shows_an_error_with_try_again() {
    let e = EngageHarness::new(vec![Err("offline".into()), Ok(page(10, 2))]);
    e.open();
    assert_eq!(es(e.app()).get_error(), "Could not load the posts: offline");
    assert!(
        has_text(e.app(), "Error: Could not load the posts: offline"),
        "the error is shown"
    );
    assert!(es(e.app()).get_posts().row_count() == 0);
    assert!(
        !has_text(e.app(), "No posts yet"),
        "an error is not an empty feed"
    );
    assert!(buttons(e.app(), "Older posts").is_empty());
    click(&buttons(e.app(), "Try again")[0]);
    assert_eq!(e.log.borrow().last().unwrap(), "refresh");
    assert_eq!(es(e.app()).get_error(), "");
    assert_eq!(posts_shown(e.app()), ["11", "10"]);
    assert!(buttons(e.app(), "Try again").is_empty());
}

#[test]
fn an_empty_community_and_a_loading_one_say_so() {
    let e = EngageHarness::new(vec![Ok(vec![])]);
    e.open();
    assert!(es(e.app()).get_posts().row_count() == 0);
    assert!(has_text(e.app(), "No posts yet"));
    assert!(!has_text(e.app(), "Loading posts"));
    es(e.app()).set_loading(true);
    assert!(has_text(e.app(), "Loading posts"));
    assert!(!has_text(e.app(), "No posts yet"));
}

#[test]
fn posting_replying_and_liking_are_disabled_and_send_nothing() {
    let e = EngageHarness::new(vec![Ok(vec![
        post(2, "Sara Example", "one"),
        post(1, "Omar Example", "two"),
    ])]);
    e.open();
    assert!(has_text(e.app(), "Posting is not available yet"));
    for label in ["Like", "Reply", "Post"] {
        let found = buttons(e.app(), label);
        let expected = if label == "Post" { 1 } else { 2 };
        assert_eq!(found.len(), expected, "{label}");
        for b in &found {
            assert_eq!(b.accessible_enabled(), Some(false), "{label} is disabled");
            click(b);
            b.invoke_accessible_default_action();
        }
    }
    // Nothing was sent: only the open, and the feed was read once.
    assert_eq!(*e.log.borrow(), vec![format!("open {COMMUNITY}")]);
    assert_eq!(e.h.engine.engage_requests().len(), 1);
}

#[test]
fn a_post_opens_in_the_browser_only_through_the_guarded_opener() {
    let e = EngageHarness::new(vec![Ok(vec![post(5, "Sara Example", "hi")])]);
    e.open();
    click(&buttons(e.app(), "Open in browser")[0]);
    assert!(
        crate::external::suppressed_targets().contains(&"https://example.test/posts/5".to_string()),
        "the target was recorded, not launched"
    );
}

#[test]
fn close_hides_the_feed_and_forgets_it() {
    let e = EngageHarness::new(vec![Ok(page(1, 2))]);
    e.open();
    assert!(es(e.app()).get_open());
    click(&buttons(e.app(), "Close community")[0]);
    assert!(!es(e.app()).get_open());
    assert_eq!(e.log.borrow().last().unwrap(), "close");
    assert_eq!(es(e.app()).get_posts().row_count(), 0);
    assert_eq!(es(e.app()).get_community(), "");
    assert!(buttons(e.app(), "Refresh").is_empty(), "the view is gone");
}

#[test]
fn the_feed_does_not_cover_other_views() {
    let e = EngageHarness::new(vec![Ok(page(1, 2))]);
    e.open();
    assert!(!buttons(e.app(), "Refresh").is_empty());
    e.app().set_view("calendar".into());
    assert!(buttons(e.app(), "Refresh").is_empty());
}
