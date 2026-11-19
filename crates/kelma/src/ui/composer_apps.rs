//! UI-thread side of the composer's apps picker: fills the `ComposerApps` global.

use super::{UiHandle, img, model};
use crate::backend::composer_apps::PickerData;
use crate::backend::{Cmd, ComposerAppsCmd};
use crate::{AppWindow, CaCard, CaCommand, CaResult, ComposerApps};
use slint::ComponentHandle;
use tokio::sync::mpsc::UnboundedSender;

fn set(w: &AppWindow, d: PickerData) {
    let s = w.global::<ComposerApps>();
    s.set_open(d.open);
    s.set_commands(model(
        d.commands
            .into_iter()
            .map(|c| CaCommand {
                key: c.key.into(),
                app: c.app.into(),
                title: c.title.into(),
                description: c.description.into(),
                action: c.action,
                header: c.header,
            })
            .collect(),
    ));
    s.set_command_key(d.command_key.into());
    s.set_command_title(d.command_title.into());
    s.set_app_name(d.app_name.into());
    s.set_placeholder(d.placeholder.into());
    s.set_loading(d.loading);
    s.set_searching(d.searching);
    s.set_status(d.status.into());
    s.set_results(model(
        d.results
            .into_iter()
            .map(|r| {
                let (thumb, has_thumb) = img(&r.thumb);
                CaResult {
                    title: r.title.into(),
                    subtitle: r.subtitle.into(),
                    thumb,
                    has_thumb,
                    letter: r.letter.into(),
                }
            })
            .collect(),
    ));
    s.set_cards(model(
        d.cards
            .into_iter()
            .map(|c| CaCard {
                app: c.app.into(),
                title: c.title.into(),
                subtitle: c.subtitle.into(),
            })
            .collect(),
    ));
    s.set_note(d.note.into());
}

impl UiHandle {
    pub fn composer_apps(&self, d: PickerData) {
        self.run(move |w| set(&w, d));
    }
}

/// Connects the picker's `command` callback to the engine's command channel.
pub fn wire(window: &AppWindow, tx: UnboundedSender<Cmd>) {
    window
        .global::<ComposerApps>()
        .on_command(move |name, arg| match ComposerAppsCmd::parse(&name, &arg) {
            Some(c) => {
                let _ = tx.send(Cmd::ComposerApps(c));
            }
            None => tracing::warn!(%name, "unknown composer apps command"),
        });
}
