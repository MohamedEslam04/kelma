//! The top search box's local side: suggestions while typing, and `/goto`, `/chat` and `/call`.
//! Both look only at the cached conversations (`crate::suggest`); nothing here asks the server.

use crate::backend::Engine;
use crate::suggest::{self, Target};
use std::sync::Arc;
use store::ConvFilter;

/// More than anyone has: the cache is scanned in full.
const ALL: usize = 5000;

impl Engine {
    /// The suggestions for what is typed in the search box.
    pub(super) fn suggest_start(self: &Arc<Self>, query: String) {
        let convs = self
            .store
            .lock()
            .unwrap()
            .conversations(ConvFilter::All, ALL)
            .unwrap_or_default();
        let rows = suggest::suggest(&convs, &query);
        self.ui.topbar_suggestions(query, rows);
    }

    /// `/goto`, `/chat` and `/call`: `arg` is the command's name, U+001F and what was typed.
    pub(super) async fn go_to(self: &Arc<Self>, arg: &str) -> anyhow::Result<()> {
        let (command, name) = arg.split_once('\u{1f}').unwrap_or((arg, ""));
        let Some(target) = Target::parse(command) else {
            return Ok(());
        };
        let found = {
            let convs = self
                .store
                .lock()
                .unwrap()
                .conversations(ConvFilter::All, ALL)?;
            suggest::resolve(&convs, target, name.trim()).cloned()
        };
        let Some(conv) = found else {
            let what = match target {
                Target::Channel => "channel or team",
                Target::Chat => "chat",
                Target::Person => "person",
            };
            self.ui.banner(format!(
                "No {what} called \"{}\" in your chats.",
                name.trim()
            ));
            return Ok(());
        };
        match target {
            Target::Channel => {
                self.ui.show_view("teams");
                self.open(conv.id, Some(conv.title)).await?;
            }
            Target::Chat => {
                self.ui.show_view("chat");
                self.open(conv.id, None).await?;
            }
            Target::Person => self.call_person(conv.id).await,
        }
        Ok(())
    }

    #[cfg(feature = "calls")]
    async fn call_person(self: &Arc<Self>, chat_id: String) {
        self.clone()
            .handle(crate::backend::Cmd::DirectCall(
                crate::backend::DirectCallCmd::DialChat(chat_id),
            ))
            .await;
    }

    #[cfg(not(feature = "calls"))]
    async fn call_person(self: &Arc<Self>, _chat_id: String) {
        self.ui.banner("This build of kelma has no calls.".into());
    }
}
