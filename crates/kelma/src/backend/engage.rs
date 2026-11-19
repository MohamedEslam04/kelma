//! Viva Engage communities: the read-only feed of one community (plan `plans/store-and-apps.md`
//! S3.2).
//!
//! The UI sends `EngageStore.command(name, argument)`; [`EngageCmd::parse`] turns that into a
//! command and `Engine::engage_cmd` runs it, then pushes the whole view as one [`EngageData`].
//!
//! Read only by construction: the only calls made are `Client::communities` (for the title) and
//! `Client::community_feed`, both GETs. There is no command for posting, replying or liking, and
//! storyline posts (`Client::post_storyline`) are blocked by plan T2.9 and never called here.

use super::Engine;
use super::util::chat_time;
use std::collections::HashMap;
use std::sync::Arc;
use teams_proto::engage::{Community, EngagePost};

/// Posts per page.
const PAGE: u32 = 20;

/// What the UI can ask for. None of these writes.
#[derive(Debug, Clone, PartialEq)]
pub enum EngageCmd {
    /// Open a community and load its newest posts.
    Open(u64),
    /// Reload the open community's newest posts.
    Refresh,
    /// Load the page older than the oldest post shown.
    Older,
    /// Forget the open community.
    Close,
    /// Open a post's web page in the browser.
    OpenWeb(String),
}

impl EngageCmd {
    /// The commands the UI sends as `(name, argument)`.
    pub fn parse(name: &str, arg: &str) -> Option<EngageCmd> {
        Some(match name {
            "open" => EngageCmd::Open(arg.parse().ok()?),
            "refresh" => EngageCmd::Refresh,
            "older" => EngageCmd::Older,
            "close" => EngageCmd::Close,
            "open-web" if arg.starts_with("https://") => EngageCmd::OpenWeb(arg.to_string()),
            _ => return None,
        })
    }
}

/// One post row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PostData {
    pub id: String,
    pub sender: String,
    pub initials: String,
    pub time: String,
    pub text: String,
    pub likes: u32,
    pub replies: u32,
    pub attachments: u32,
    pub url: String,
    /// A reply whose original post is not among the loaded pages.
    pub reply: bool,
}

/// What the Engage view shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngageData {
    pub community: String,
    pub title: String,
    pub posts: Vec<PostData>,
    pub more: bool,
    pub loading: bool,
    pub error: String,
}

/// The text of a post: the plain body, else the text of the rich one.
fn body_text(p: &EngagePost) -> String {
    let plain = p.text.trim();
    if !plain.is_empty() {
        return plain.to_string();
    }
    if p.rich.trim().is_empty() {
        return String::new();
    }
    render::plain_text(&render::parse_teams_html(&p.rich))
        .trim()
        .to_string()
}

/// The rows of the loaded posts, in feed order. A thread's replies are counted on its first post
/// (the larger of the loaded replies and what the thread says it holds) and not listed, except
/// replies whose first post is not loaded: those are listed so a page of replies is not blank.
pub(crate) fn rows(posts: &[EngagePost]) -> Vec<PostData> {
    let mut loaded_replies: HashMap<u64, u32> = HashMap::new();
    for p in posts.iter().filter(|p| p.replied_to_id.is_some()) {
        *loaded_replies.entry(p.thread_id).or_default() += 1;
    }
    let has_first = |thread: u64| {
        posts
            .iter()
            .any(|p| p.id == thread && p.replied_to_id.is_none())
    };
    posts
        .iter()
        .filter(|p| p.replied_to_id.is_none() || !has_first(p.thread_id))
        .map(|p| {
            let reply = p.replied_to_id.is_some();
            let sender = if p.sender_name.is_empty() {
                "Unknown sender".to_string()
            } else {
                p.sender_name.clone()
            };
            PostData {
                id: p.id.to_string(),
                initials: crate::data::initials(&sender),
                sender,
                time: chat_time(p.created_ms),
                text: body_text(p),
                likes: p.likes,
                replies: if reply {
                    0
                } else {
                    loaded_replies
                        .get(&p.thread_id)
                        .copied()
                        .unwrap_or(0)
                        .max(p.thread_messages.saturating_sub(1))
                },
                attachments: p.attachments,
                url: p.web_url.clone(),
                reply,
            }
        })
        .collect()
}

/// How a loaded page joins the posts.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// The newest page: replaces what is shown.
    Replace,
    /// An older page: added below.
    Append,
}

/// A scripted feed for tests: the communities, and the answers to successive feed requests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Fake {
    pub communities: Vec<Community>,
    pub pages: std::collections::VecDeque<Result<Vec<EngagePost>, String>>,
    /// `(community, older_than)` of every feed request made.
    pub requests: Vec<(u64, Option<u64>)>,
}

#[derive(Default)]
pub(crate) struct State {
    names: HashMap<u64, String>,
    community: Option<u64>,
    title: String,
    posts: Vec<EngagePost>,
    more: bool,
    loading: bool,
    error: String,
    /// Bumped by every load, so an answer to an earlier one is dropped.
    generation: u64,
    #[cfg(test)]
    pub(crate) fake: Option<Fake>,
}

impl State {
    /// Starts showing a community: its posts are cleared until the first page arrives.
    fn open(&mut self, id: u64) -> u64 {
        self.community = Some(id);
        self.title = self.names.get(&id).cloned().unwrap_or_default();
        self.posts.clear();
        self.more = false;
        self.begin()
    }

    /// Starts a load; returns its generation.
    fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.loading = true;
        self.error.clear();
        self.generation
    }

    /// The id to ask for the next older page: the oldest post loaded.
    fn oldest(&self) -> Option<u64> {
        self.posts.iter().map(|p| p.id).min()
    }

    /// Takes a page or an error in. Returns false for the answer to a superseded load.
    fn apply(
        &mut self,
        generation: u64,
        mode: Mode,
        result: Result<Vec<EngagePost>, String>,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.loading = false;
        match result {
            Ok(page) => {
                self.more = page.len() as u32 >= PAGE;
                match mode {
                    Mode::Replace => self.posts = page,
                    Mode::Append => {
                        for p in page {
                            if !self.posts.iter().any(|x| x.id == p.id) {
                                self.posts.push(p);
                            }
                        }
                    }
                }
            }
            Err(e) => self.error = e,
        }
        true
    }

    fn data(&self) -> EngageData {
        EngageData {
            community: self.community.map(|c| c.to_string()).unwrap_or_default(),
            title: self.title.clone(),
            posts: rows(&self.posts),
            more: self.more && self.error.is_empty(),
            loading: self.loading,
            error: self.error.clone(),
        }
    }
}

impl Engine {
    fn engage_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.engage.lock().unwrap())
    }

    fn engage_push(&self) {
        let d = self.engage_state(|s| s.data());
        self.ui.engage_store(d);
    }

    /// The communities (read only), for the title of the open one.
    async fn engage_fetch_names(&self) -> Result<Vec<Community>, String> {
        #[cfg(test)]
        if let Some(list) = self.engage_state(|s| s.fake.as_ref().map(|f| f.communities.clone())) {
            return Ok(list);
        }
        self.client.communities().await.map_err(|e| e.to_string())
    }

    /// One page of a community's feed (read only).
    async fn engage_fetch_page(
        &self,
        id: u64,
        older_than: Option<u64>,
    ) -> Result<Vec<EngagePost>, String> {
        #[cfg(test)]
        if let Some(r) = self.engage_state(|s| {
            s.fake.as_mut().map(|f| {
                f.requests.push((id, older_than));
                f.pages.pop_front().unwrap_or_else(|| Ok(vec![]))
            })
        }) {
            return r;
        }
        self.client
            .community_feed(id, PAGE, older_than)
            .await
            .map_err(|e| e.to_string())
    }

    /// Loads one page for the load that has `generation` and pushes the result.
    async fn engage_load(&self, generation: u64, id: u64, older_than: Option<u64>, mode: Mode) {
        self.engage_push();
        let result = self.engage_fetch_page(id, older_than).await;
        if let Err(err) = &result {
            tracing::warn!(%err, "community feed failed");
        }
        let result = result.map_err(|e| format!("Could not load the posts: {e}"));
        if self.engage_state(|s| s.apply(generation, mode, result)) {
            self.engage_push();
        }
    }

    pub(super) async fn engage_cmd(self: &Arc<Self>, c: EngageCmd) -> anyhow::Result<()> {
        match c {
            EngageCmd::Open(id) => {
                let known = self.engage_state(|s| s.names.contains_key(&id));
                let generation = self.engage_state(|s| s.open(id));
                self.engage_push();
                if !known && let Ok(list) = self.engage_fetch_names().await {
                    self.engage_state(|s| {
                        s.names = list.into_iter().map(|c| (c.id, c.name)).collect();
                        if s.community == Some(id) {
                            s.title = s.names.get(&id).cloned().unwrap_or_default();
                        }
                    });
                }
                self.engage_load(generation, id, None, Mode::Replace).await;
            }
            EngageCmd::Refresh => {
                if let Some((id, generation)) = self.engage_state(|s| {
                    let id = s.community?;
                    (!s.loading).then(|| (id, s.begin()))
                }) {
                    self.engage_load(generation, id, None, Mode::Replace).await;
                }
            }
            EngageCmd::Older => {
                if let Some((id, older, generation)) = self.engage_state(|s| {
                    let id = s.community?;
                    let older = s.oldest()?;
                    (!s.loading && s.more).then(|| (id, older, s.begin()))
                }) {
                    self.engage_load(generation, id, Some(older), Mode::Append)
                        .await;
                }
            }
            EngageCmd::Close => {
                self.engage_state(|s| {
                    s.community = None;
                    s.posts.clear();
                    s.more = false;
                    s.loading = false;
                    s.error.clear();
                    s.generation += 1;
                });
                self.engage_push();
            }
            EngageCmd::OpenWeb(url) => {
                let _ = crate::external::open(url);
            }
        }
        Ok(())
    }

    /// Scripts the feed for a test.
    #[cfg(test)]
    pub(crate) fn engage_script(&self, fake: Fake) {
        self.engage_state(|s| s.fake = Some(fake));
    }

    /// The `(community, older_than)` requests the scripted feed has seen (tests).
    #[cfg(test)]
    pub(crate) fn engage_requests(&self) -> Vec<(u64, Option<u64>)> {
        self.engage_state(|s| {
            s.fake
                .as_ref()
                .map(|f| f.requests.clone())
                .unwrap_or_default()
        })
    }
}

#[cfg(test)]
pub(crate) fn fake_post(id: u64, thread: u64, replied_to: Option<u64>, text: &str) -> EngagePost {
    EngagePost {
        id,
        thread_id: thread,
        replied_to_id: replied_to,
        sender_id: 7,
        sender_name: "Sara Example".into(),
        created_ms: 1_700_000_000_000 + id as i64,
        text: text.into(),
        rich: String::new(),
        likes: 0,
        attachments: 0,
        thread_messages: 0,
        web_url: format!("https://example.test/posts/{id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_read_commands() {
        assert_eq!(EngageCmd::parse("open", "42"), Some(EngageCmd::Open(42)));
        assert_eq!(EngageCmd::parse("open", "x"), None);
        assert_eq!(EngageCmd::parse("older", ""), Some(EngageCmd::Older));
        assert_eq!(EngageCmd::parse("refresh", ""), Some(EngageCmd::Refresh));
        assert_eq!(
            EngageCmd::parse("open-web", "https://example.test/p"),
            Some(EngageCmd::OpenWeb("https://example.test/p".into()))
        );
        assert_eq!(EngageCmd::parse("open-web", "file:///etc/passwd"), None);
        assert_eq!(EngageCmd::parse("open-web", "javascript:alert(1)"), None);
        for write in [
            "post",
            "reply",
            "like",
            "react",
            "storyline",
            "delete",
            "send",
        ] {
            assert_eq!(EngageCmd::parse(write, "1"), None, "{write} must not parse");
        }
    }

    #[test]
    fn replies_are_counted_on_their_first_post_and_not_listed() {
        let mut first = fake_post(10, 10, None, "Hello");
        first.likes = 4;
        first.attachments = 2;
        let posts = vec![
            fake_post(12, 10, Some(10), "second reply"),
            fake_post(11, 10, Some(10), "first reply"),
            first,
            fake_post(9, 9, None, "Other"),
        ];
        let r = rows(&posts);
        assert_eq!(
            r.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["10", "9"]
        );
        assert_eq!(r[0].replies, 2);
        assert_eq!((r[0].likes, r[0].attachments), (4, 2));
        assert_eq!(r[1].replies, 0);
        assert_eq!(r[0].sender, "Sara Example");
        assert_eq!(r[0].initials, "SE");
        assert!(!r[0].reply);
    }

    #[test]
    fn the_thread_statistics_count_replies_that_are_not_loaded() {
        let mut first = fake_post(10, 10, None, "Hello");
        first.thread_messages = 6;
        let r = rows(&[first]);
        assert_eq!(r[0].replies, 5);
        // The larger of the two counts wins.
        let mut first = fake_post(10, 10, None, "Hello");
        first.thread_messages = 2;
        let r = rows(&[
            fake_post(11, 10, Some(10), "a"),
            fake_post(12, 10, Some(10), "b"),
            first,
        ]);
        assert_eq!(r[0].replies, 2);
    }

    #[test]
    fn a_reply_without_its_first_post_is_listed_as_a_reply() {
        let r = rows(&[
            fake_post(11, 10, Some(10), "orphan"),
            fake_post(5, 5, None, "x"),
        ]);
        assert_eq!(r.len(), 2);
        assert!(r[0].reply);
        assert_eq!(r[0].replies, 0);
        assert!(!r[1].reply);
    }

    #[test]
    fn a_post_without_a_known_sender_says_so() {
        let mut p = fake_post(1, 1, None, "hi");
        p.sender_name.clear();
        let r = rows(&[p]);
        assert_eq!(r[0].sender, "Unknown sender");
    }

    #[test]
    fn the_body_is_plain_text_else_the_text_of_the_rich_body() {
        let mut p = fake_post(1, 1, None, "  plain  ");
        p.rich = "<b>rich</b>".into();
        assert_eq!(body_text(&p), "plain");
        p.text.clear();
        assert_eq!(body_text(&p), "rich");
        p.rich.clear();
        assert_eq!(body_text(&p), "");
        p.rich = "<p>one</p><p>two <a href=\"https://example.test\">link</a></p>".into();
        assert!(body_text(&p).contains("one") && body_text(&p).contains("two link"));
    }

    fn page(from: u64, n: u64) -> Vec<EngagePost> {
        (from..from + n)
            .rev()
            .map(|i| fake_post(i, i, None, "p"))
            .collect()
    }

    #[test]
    fn state_pages_in_older_posts_without_duplicates() {
        let mut s = State::default();
        let g = s.open(1);
        assert!(s.loading && s.posts.is_empty());
        assert!(s.apply(g, Mode::Replace, Ok(page(100, 20))));
        assert!(s.more && !s.loading);
        assert_eq!(s.oldest(), Some(100));
        let g = s.begin();
        // One post of the new page is already shown.
        let mut older = page(80, 20);
        older.push(fake_post(100, 100, None, "p"));
        assert!(s.apply(g, Mode::Append, Ok(older)));
        assert_eq!(s.posts.len(), 40);
        assert_eq!(s.oldest(), Some(80));
        let g = s.begin();
        assert!(s.apply(g, Mode::Append, Ok(page(70, 3))));
        assert!(!s.more, "a short page is the last one");
    }

    #[test]
    fn a_stale_answer_is_dropped_and_an_error_keeps_the_posts() {
        let mut s = State::default();
        let first = s.open(1);
        let second = s.open(2);
        assert!(!s.apply(first, Mode::Replace, Ok(page(1, 3))));
        assert!(s.posts.is_empty() && s.loading);
        assert!(s.apply(second, Mode::Replace, Ok(page(10, 3))));
        let g = s.begin();
        assert!(s.apply(g, Mode::Replace, Err("offline".into())));
        assert_eq!(s.posts.len(), 3);
        assert_eq!(s.data().error, "offline");
        assert!(!s.data().more, "no pager next to an error");
        let g = s.begin();
        assert!(s.error.is_empty(), "a new load clears the error");
        assert!(s.apply(g, Mode::Replace, Ok(page(10, 5))));
        assert_eq!(s.posts.len(), 5);
    }

    /// Live, read only: lists the communities and loads one feed page and one older page. Only
    /// counts are printed. Run by hand:
    /// `cargo test -p kelma engage_live -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "needs the signed-in account"]
    async fn engage_live() {
        let paths = session::Paths::new().unwrap();
        let cfg = paths.load_config().unwrap();
        let mut sess = None;
        for _ in 0..4 {
            match session::stored_session(&cfg) {
                Ok(s) => {
                    sess = s;
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_secs(5)).await,
            }
        }
        let client = teams_proto::Client::resume(sess.expect("signed in"))
            .await
            .unwrap();
        // Two GETs only: the communities and one feed page.
        let list = client.communities().await.unwrap();
        println!("communities: {}", list.len());
        let Some(first) = list.first() else { return };
        let posts = client.community_feed(first.id, PAGE, None).await.unwrap();
        let roots = posts.iter().filter(|p| p.replied_to_id.is_none()).count();
        println!(
            "posts: {}, first posts: {roots}, replies: {}, with sender name: {}, with plain body: {}, rich only: {}, with thread stats: {}",
            posts.len(),
            posts.len() - roots,
            posts.iter().filter(|p| !p.sender_name.is_empty()).count(),
            posts.iter().filter(|p| !p.text.trim().is_empty()).count(),
            posts
                .iter()
                .filter(|p| p.text.trim().is_empty() && !p.rich.trim().is_empty())
                .count(),
            posts.iter().filter(|p| p.thread_messages > 0).count(),
        );
        let r = rows(&posts);
        println!(
            "rows: {}, with replies: {}, with text: {}",
            r.len(),
            r.iter().filter(|p| p.replies > 0).count(),
            r.iter().filter(|p| !p.text.is_empty()).count()
        );
    }
}
