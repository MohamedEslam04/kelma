//! UI-thread side of the Viva Engage feed: fills the `EngageStore` global.

use super::{UiHandle, model};
use crate::backend::engage::EngageData;
use crate::{AppWindow, EPost, EngageStore};
use slint::ComponentHandle;

fn int(n: impl TryInto<i32>) -> i32 {
    n.try_into().unwrap_or(i32::MAX)
}

fn set(w: &AppWindow, d: EngageData) {
    let s = w.global::<EngageStore>();
    s.set_community(d.community.into());
    s.set_title(d.title.into());
    s.set_posts(model(
        d.posts
            .into_iter()
            .map(|p| EPost {
                id: p.id.into(),
                sender: p.sender.into(),
                initials: p.initials.into(),
                time: p.time.into(),
                text: p.text.into(),
                likes: int(p.likes),
                replies: int(p.replies),
                attachments: int(p.attachments),
                url: p.url.into(),
                reply: p.reply,
            })
            .collect(),
    ));
    s.set_more(d.more);
    s.set_loading(d.loading);
    s.set_error(d.error.into());
}

impl UiHandle {
    /// Replaces everything the Engage feed shows.
    pub fn engage_store(&self, d: EngageData) {
        self.run(move |w| set(&w, d));
    }
}
