//! UI-thread side of the reminders panel: fills the `RemindStore` global.

use super::{UiHandle, model};
use crate::backend::{ReminderData, SnoozedData};
use crate::{AppWindow, RemindStore, ReminderRow, SnoozedRow};
use slint::ComponentHandle;

impl UiHandle {
    /// The pending reminders and the snoozed chats, soonest first.
    pub fn reminders(&self, reminders: Vec<ReminderData>, snoozed: Vec<SnoozedData>) {
        self.run(move |w: AppWindow| {
            let s = w.global::<RemindStore>();
            s.set_reminders(model(
                reminders
                    .into_iter()
                    .map(|r| ReminderRow {
                        id: r.id.into(),
                        time: r.when.into(),
                        chat: r.chat.into(),
                        text: r.text.into(),
                    })
                    .collect(),
            ));
            s.set_snoozed(model(
                snoozed
                    .into_iter()
                    .map(|c| SnoozedRow {
                        id: c.id.into(),
                        title: c.title.into(),
                        until: c.until.into(),
                    })
                    .collect(),
            ));
        });
    }
}
