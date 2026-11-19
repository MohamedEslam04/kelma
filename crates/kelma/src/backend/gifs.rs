//! The composer's GIF picker: search (trending when empty) through Teams' Giphy extension,
//! small previews for the grid, and the picked GIF sent to the open chat, thread or channel.

use super::Engine;
use super::util::decode_image;
use crate::data::Pixels;
use std::sync::{Arc, Mutex};
use store::OutboxKind;
use teams_proto::gifs::{Gif, gif_html};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GifCmd {
    /// Searches; an empty query shows the trending GIFs.
    Search(String),
    /// Sends the result at this index.
    Pick(usize),
}

impl GifCmd {
    pub fn parse(name: &str, arg: &str) -> Option<GifCmd> {
        Some(match name {
            "search" => GifCmd::Search(arg.to_string()),
            "pick" => GifCmd::Pick(arg.parse().ok()?),
            _ => return None,
        })
    }
}

/// What the picker shows: one row per result.
#[derive(Clone, Default)]
pub struct GifRow {
    pub title: String,
    pub preview: Option<Pixels>,
}

#[derive(Default)]
struct State {
    /// Bumped by every search; a slower earlier answer is dropped.
    seq: u64,
    results: Vec<Gif>,
    previews: Vec<Option<Pixels>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = STATE.lock().unwrap();
    f(g.get_or_insert_with(State::default))
}

fn rows() -> Vec<GifRow> {
    with(|s| {
        s.results
            .iter()
            .zip(s.previews.iter())
            .map(|(g, p)| GifRow {
                title: g.title.clone(),
                preview: p.clone(),
            })
            .collect()
    })
}

/// Where a picked GIF goes: the open thread, else the open chat or channel.
pub fn target(conv: &str, thread_root: Option<&str>) -> String {
    match thread_root {
        Some(root) => format!("{conv};messageid={root}"),
        None => conv.to_string(),
    }
}

impl Engine {
    pub(super) async fn gif_cmd(self: &Arc<Self>, cmd: GifCmd) -> anyhow::Result<()> {
        match cmd {
            GifCmd::Search(q) => self.gif_search(q).await,
            GifCmd::Pick(i) => self.gif_pick(i).await,
        }
    }

    async fn gif_search(self: &Arc<Self>, q: String) -> anyhow::Result<()> {
        let seq = with(|s| {
            s.seq += 1;
            s.seq
        });
        if self.offline.load(std::sync::atomic::Ordering::Relaxed) {
            self.ui.gifs(vec![], "GIFs need a connection.".into());
            return Ok(());
        }
        self.ui.gifs(rows(), "Searching…".into());
        let found = match self.client.search_gifs(q.trim(), "en-us").await {
            Ok(f) => f,
            Err(e) => {
                self.ui.gifs(vec![], format!("Could not search GIFs: {e}"));
                return Ok(());
            }
        };
        let urls: Vec<String> = found.iter().map(|g| g.preview_url.clone()).collect();
        let fresh = with(|s| {
            if s.seq != seq {
                return false;
            }
            s.previews = vec![None; found.len()];
            s.results = found;
            true
        });
        if !fresh {
            return Ok(());
        }
        let status = if urls.is_empty() {
            "No GIFs found."
        } else {
            ""
        };
        self.ui.gifs(rows(), status.into());
        // Previews arrive one by one; each redraws the grid.
        for (i, url) in urls.into_iter().enumerate() {
            let e = self.clone();
            tokio::spawn(async move {
                let Ok(bytes) = e.client.bytes(&url, &[], 4 * 1024 * 1024).await else {
                    return;
                };
                let px = tokio::task::spawn_blocking(move || decode_image(&bytes, 160, 120))
                    .await
                    .ok()
                    .flatten();
                let current = with(|s| {
                    if s.seq != seq {
                        return false;
                    }
                    if let Some(p) = s.previews.get_mut(i) {
                        *p = px;
                    }
                    true
                });
                if current {
                    e.ui.gifs(rows(), String::new());
                }
            });
        }
        Ok(())
    }

    async fn gif_pick(self: &Arc<Self>, i: usize) -> anyhow::Result<()> {
        let Some(gif) = with(|s| s.results.get(i).cloned()) else {
            return Ok(());
        };
        let (conv, root) = {
            let v = self.view.lock().unwrap();
            (v.current.clone(), v.thread_root.clone())
        };
        let Some(conv) = conv else {
            return Ok(());
        };
        let item = self.new_outbox_item(
            target(&conv, root.as_deref()),
            OutboxKind::Text,
            gif_html(&gif),
            None,
            None,
            None,
        );
        self.enqueue(item)?;
        self.flush_outbox(false).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse() {
        assert_eq!(
            GifCmd::parse("search", "cats"),
            Some(GifCmd::Search("cats".into()))
        );
        assert_eq!(GifCmd::parse("pick", "3"), Some(GifCmd::Pick(3)));
        assert_eq!(GifCmd::parse("pick", "x"), None);
    }

    #[test]
    fn a_gif_in_a_thread_goes_to_the_thread() {
        assert_eq!(target("19:c@thread.tacv2", None), "19:c@thread.tacv2");
        assert_eq!(
            target("19:c@thread.tacv2", Some("17000")),
            "19:c@thread.tacv2;messageid=17000"
        );
    }
}
