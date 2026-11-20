//! The Storyline tab of a 1:1 chat (and of the self chat): the person's Viva Engage card
//! (photo, name, Followers / Following / Posts) and their newest posts. Read only, like the
//! Communities feed: posting to a storyline is blocked (plan T2.9, docs/research/storyline.md),
//! so the UI draws its write controls disabled.
//!
//! The card shows at once from what kelma knows (name, photo). The counts and posts come from
//! two Engage reads found by the person's email, both UNVERIFIED (docs/protocol.md):
//! `Client::engage_user` and `Client::engage_user_posts`. Offline, or when a read fails, the card
//! stays and the tab says why.

use super::Engine;
use super::util::one_on_one_members;
use crate::data::{Pixels, color_for, initials};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use store::ConvKind;

/// Posts asked for.
const POSTS: u32 = 20;

/// What the Storyline tab shows.
#[derive(Clone, Default)]
pub struct StorylineData {
    pub name: String,
    pub initials: String,
    pub color: u32,
    pub photo: Option<Pixels>,
    /// `None` until Engage answers.
    pub followers: Option<u32>,
    pub following: Option<u32>,
    pub posts_count: Option<u32>,
    pub posts: Vec<super::engage::PostData>,
    pub loading: bool,
    /// Why the counts and posts are missing ("" when they are not).
    pub note: String,
}

impl Engine {
    /// The person a chat's Storyline tab is about: the other member of a 1:1 chat, or me in the
    /// self chat. Other chats have no Storyline tab.
    fn storyline_person(&self, conv: &str) -> Option<String> {
        let kind = self
            .store
            .lock()
            .unwrap()
            .conversation(conv)
            .ok()
            .flatten()
            .map(|c| c.kind);
        match kind {
            Some(ConvKind::Notes) => Some(self.me.clone()),
            Some(ConvKind::OneOnOne) => {
                one_on_one_members(conv).into_iter().find(|m| m != &self.me)
            }
            _ => None,
        }
    }

    pub(super) async fn storyline_open(self: &Arc<Self>) {
        let Some(conv) = self.view.lock().unwrap().current.clone() else {
            return;
        };
        let Some(mri) = self.storyline_person(&conv) else {
            return;
        };
        let user = self.store.lock().unwrap().user(&mri).ok().flatten();
        let name = user
            .as_ref()
            .map(|u| u.display_name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                if mri == self.me {
                    self.my_name.clone()
                } else {
                    "Unknown person".to_string()
                }
            });
        let email = user.and_then(|u| u.email).filter(|e| !e.is_empty());
        let offline = self.offline.load(Ordering::Relaxed);
        let mut data = StorylineData {
            initials: initials(&name),
            color: color_for(&mri),
            photo: self.avatar(&mri),
            name,
            loading: !offline && email.is_some(),
            note: if offline {
                "You're offline. Storyline posts load when kelma is connected.".into()
            } else if email.is_none() {
                "This person's Viva Engage profile can't be found.".into()
            } else {
                String::new()
            },
            ..Default::default()
        };
        self.ui.storyline(data.clone());
        let Some(email) = email.filter(|_| !offline) else {
            return;
        };
        let answer = async {
            let Some(u) = self.client.engage_user(&email).await? else {
                return Ok(None);
            };
            let posts = self.client.engage_user_posts(u.id, POSTS).await?;
            Ok::<_, teams_proto::Error>(Some((u, posts)))
        }
        .await;
        // The chat or the tab changed meanwhile.
        if self.view.lock().unwrap().current.as_deref() != Some(conv.as_str()) {
            return;
        }
        data.loading = false;
        match answer {
            Ok(Some((u, posts))) => {
                data.followers = Some(u.followers);
                data.following = Some(u.following);
                data.posts_count = Some(u.posts);
                data.posts = super::engage::rows(&posts);
            }
            Ok(None) => data.note = "This person's Viva Engage profile can't be found.".into(),
            Err(err) => {
                tracing::warn!(%err, "reading the storyline failed");
                data.note = format!("Couldn't load the storyline: {err}");
            }
        }
        self.ui.storyline(data);
    }
}
