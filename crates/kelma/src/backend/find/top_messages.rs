//! The Messages group of the top search box's dropdown. The window asks once typing has paused
//! (about 150 ms): the cache's full-text index answers at once, then, for a query of two or more
//! characters that is still the newest after a further pause, the server search (the one the
//! Search page uses) adds its top hits. A generation counter drops any answer that a newer query
//! overtook, so out-of-order replies never replace what the box shows now.

use super::{Hit, merge_messages};
use crate::backend::Engine;
use crate::find_data::{HitData, terms_of};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use store::LocalFilters;
use teams_proto::search::SearchFilters;

/// Message hits the dropdown shows.
pub(super) const TOP_MAX: usize = 6;
/// Shorter queries are answered by the cache only.
const SERVER_MIN_CHARS: usize = 2;
/// Added to the window's 150 ms pause before the server is asked: about 350 ms without typing.
const SERVER_PAUSE: Duration = Duration::from_millis(200);

/// The message hits of the newest query and its generation.
#[derive(Default)]
pub(super) struct TopMessages {
    generation: u64,
    pub(super) hits: Vec<Hit>,
}

impl TopMessages {
    /// A new query: its cache hits replace the old ones. Its generation.
    pub(super) fn start(&mut self, local: Vec<Hit>) -> u64 {
        self.generation += 1;
        self.hits = local;
        self.hits.truncate(TOP_MAX);
        self.generation
    }

    pub(super) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    /// The server's hits for the query of `generation`, merged without repeating a message and
    /// newest first. `false`, and nothing changes, when a newer query started since.
    pub(super) fn add_server(&mut self, generation: u64, server: Vec<Hit>) -> bool {
        if !self.is_current(generation) {
            return false;
        }
        merge_messages(&mut self.hits, server);
        self.hits.truncate(TOP_MAX);
        true
    }
}

impl Engine {
    /// The message hits for what is typed in the search box: the cache's now, the server's when
    /// they come.
    pub(super) fn suggest_messages(self: &Arc<Self>, query: String) {
        let q = query.trim().to_string();
        let local: Vec<Hit> = if terms_of(&q).is_empty() {
            vec![]
        } else {
            let found = self
                .store
                .lock()
                .unwrap()
                .search_messages_local(&q, &LocalFilters::default(), TOP_MAX)
                .unwrap_or_default();
            found.iter().map(|h| self.hit_from_local(h)).collect()
        };
        let generation = self.find.lock().unwrap().top.start(local);
        self.push_top(&query);
        if self.offline.load(Ordering::Relaxed) || q.chars().count() < SERVER_MIN_CHARS {
            return;
        }
        // Off the find lane: the pause and the request must not hold up other commands.
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SERVER_PAUSE).await;
            // Typing went on: a newer query has its own turn.
            if !me.find.lock().unwrap().top.is_current(generation) {
                return;
            }
            let page = match me
                .client
                .search_messages(&me.me, &q, &SearchFilters::default(), 0, TOP_MAX as u32)
                .await
            {
                Ok(p) => p,
                Err(err) => {
                    tracing::debug!(%err, "top search: server message search failed");
                    return;
                }
            };
            let hits: Vec<Hit> = page.hits.iter().map(|h| me.hit_from_message(h)).collect();
            if me.find.lock().unwrap().top.add_server(generation, hits) {
                me.push_top(&query);
            }
        });
    }

    /// Sends the message hits to the dropdown, for `query`.
    fn push_top(self: &Arc<Self>, query: &str) {
        let hits = self.find.lock().unwrap().top.hits.clone();
        let terms = terms_of(query);
        let rows: Vec<HitData> = hits
            .iter()
            .map(|h| self.row_data(h, String::new(), &terms))
            .collect();
        self.ui.topbar_messages(query.to_string(), rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(message: &str, ms: i64) -> Hit {
        Hit {
            kind: "message",
            conv: "19:c@thread.v2".into(),
            message: message.into(),
            time_ms: ms,
            ..Default::default()
        }
    }

    fn ids(t: &TopMessages) -> Vec<&str> {
        t.hits.iter().map(|h| h.message.as_str()).collect()
    }

    #[test]
    fn a_server_answer_for_an_older_query_is_dropped() {
        let mut t = TopMessages::default();
        let old = t.start(vec![hit("a", 10)]);
        let new = t.start(vec![hit("b", 20)]);
        // The answer to the first query comes last: it must not touch the newer hits.
        assert!(!t.add_server(old, vec![hit("x", 99)]));
        assert_eq!(ids(&t), ["b"]);
        assert!(t.add_server(new, vec![hit("y", 30)]));
        assert_eq!(ids(&t), ["y", "b"]);
    }

    #[test]
    fn server_hits_merge_without_repeats_newest_first_and_capped() {
        let mut t = TopMessages::default();
        let g = t.start((0..4).map(|i| hit(&format!("l{i}"), i)).collect());
        // `l3` is in the cache too; four new ones.
        let server = vec![
            hit("l3", 3),
            hit("s1", 100),
            hit("s2", 101),
            hit("s3", 102),
            hit("s4", 50),
        ];
        assert!(t.add_server(g, server));
        assert_eq!(ids(&t), ["s3", "s2", "s1", "s4", "l3", "l2"]);
        assert_eq!(t.hits.len(), TOP_MAX);
    }

    #[test]
    fn the_cache_hits_are_capped_too() {
        let mut t = TopMessages::default();
        t.start((0..20).map(|i| hit(&format!("m{i}"), i)).collect());
        assert_eq!(t.hits.len(), TOP_MAX);
    }
}
