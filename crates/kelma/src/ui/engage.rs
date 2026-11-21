//! UI-thread side of the Viva Engage feed: fills the `EngageStore` global.

use super::{UiHandle, img, model, rgb};
use crate::backend::engage::{EngageData, PostData};
use crate::backend::storyline::StorylineData;
use crate::{AppWindow, EPost, EngageStore, StorylineStore};
use slint::ComponentHandle;

fn int(n: impl TryInto<i32>) -> i32 {
    n.try_into().unwrap_or(i32::MAX)
}

fn post(p: PostData) -> EPost {
    EPost {
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
    }
}

fn set(w: &AppWindow, d: EngageData) {
    let s = w.global::<EngageStore>();
    s.set_community(d.community.into());
    s.set_title(d.title.into());
    s.set_posts(model(d.posts.into_iter().map(post).collect()));
    s.set_more(d.more);
    s.set_loading(d.loading);
    s.set_error(d.error.into());
}

impl UiHandle {
    /// Replaces everything the Engage feed shows.
    pub fn engage_store(&self, d: EngageData) {
        self.run(move |w| set(&w, d));
    }

    /// Replaces everything the Storyline tab shows. An unknown count is -1 (drawn as "–").
    pub fn storyline(&self, d: StorylineData) {
        self.run(move |w| {
            let s = w.global::<StorylineStore>();
            let (photo, has_photo) = img(&d.photo);
            s.set_name(d.name.into());
            s.set_initials(d.initials.into());
            s.set_color(rgb(d.color));
            s.set_photo(photo);
            s.set_has_photo(has_photo);
            s.set_followers(d.followers.map_or(-1, int));
            s.set_following(d.following.map_or(-1, int));
            s.set_posts_count(d.posts_count.map_or(-1, int));
            s.set_posts(model(d.posts.into_iter().map(post).collect()));
            s.set_loading(d.loading);
            s.set_note(d.note.into());
        });
    }
}
