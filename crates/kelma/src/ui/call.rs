//! UI-thread side of the call window: fills the `CallStore` global (`ui/call_store.slint`).

use super::{UiHandle, model};
use crate::backend::call::{CallData, is_meeting_chat};
use crate::data::initials;
use crate::{AppWindow, CallPerson, CallStore};
use slint::{ComponentHandle, SharedString};

fn int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn texts(v: Vec<String>) -> slint::ModelRc<SharedString> {
    model(v.into_iter().map(SharedString::from).collect())
}

fn set(w: &AppWindow, d: CallData) {
    let s = w.global::<CallStore>();
    if d.phase.is_empty() || d.phase == "prejoin" {
        // A new call opens full size, with no panel or menu left from the last one.
        s.set_minimized(false);
        s.set_menu(SharedString::new());
        if d.phase.is_empty() {
            s.set_panel(SharedString::new());
        }
    }
    if !d.error.is_empty() {
        // Why a join was refused shows on the full window.
        s.set_minimized(false);
    }
    s.set_phase(d.phase.into());
    s.set_kind(d.kind.into());
    s.set_title(d.title.into());
    s.set_my_initials(initials(&d.my_name).into());
    s.set_my_name(d.my_name.into());
    s.set_timer(d.timer.into());
    s.set_muted(d.muted);
    s.set_hand(d.hand);
    s.set_prejoin_mic(d.prejoin_mic);
    s.set_recording(d.recording);
    s.set_error(d.error.into());
    s.set_ended(d.ended.into());
    s.set_chat_id(d.chat_id.into());
    s.set_can_rejoin(d.can_rejoin);
    s.set_in_call(int(d.in_call));
    s.set_in_lobby(int(d.in_lobby));
    s.set_people(model(
        d.people
            .into_iter()
            .map(|p| CallPerson {
                initials: initials(&p.name).into(),
                name: p.name.into(),
                muted: p.muted,
                hand: p.hand,
                lobby: p.lobby,
                me: p.me,
                role: p.role.into(),
            })
            .collect(),
    ));
    s.set_mics(texts(d.mics));
    s.set_mic(int(d.mic));
    s.set_speakers(texts(d.speakers));
    s.set_speaker(int(d.speaker));
}

/// Wires what the call window answers by itself: whether a conversation is a meeting chat (the
/// header's Join button).
pub fn wire(window: &AppWindow) {
    window
        .global::<CallStore>()
        .on_is_meeting_chat(|id| is_meeting_chat(&id));
}

impl UiHandle {
    /// Replaces everything the call window shows.
    pub fn call_state(&self, d: CallData) {
        self.run(move |w| set(&w, d));
    }
}
