//! Loop paragraphs from the composer. A live Loop component is a `.loop` file in a SharePoint
//! Embedded container, kept in sync through Fluid; no write path for that is known (see
//! docs/research/apps-onedrive-copilot-store.md, Loop). So the paragraph is sent as a formatted
//! message instead (its HTML is built by `crate::loop_compose`), and the UI says so.

use super::Engine;
use std::sync::Arc;
use store::OutboxKind;

impl Engine {
    pub(super) async fn send_loop(self: &Arc<Self>, html: String) -> anyhow::Result<()> {
        if html.trim().is_empty() {
            return Ok(());
        }
        let Some(target) = self.current_target() else {
            return Ok(());
        };
        let item = self.new_outbox_item(target, OutboxKind::Text, html, None, None, None);
        self.enqueue(item)?;
        self.ui
            .info("Sent as a formatted message: live Loop components need Microsoft Loop.".into());
        self.flush_outbox(false).await;
        Ok(())
    }
}
