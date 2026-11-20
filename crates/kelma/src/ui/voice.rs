//! UI-thread side of voice messages: fills `VoiceStore` (the player in chat rows) and
//! `VoiceRecStore` (the recorder in the composer), and wires their callbacks to `Cmd::Voice`.

use super::{model, UiHandle};
use crate::backend::{Cmd, VoiceCmd};
use crate::{AppWindow, VoiceRecStore, VoiceStore};
use slint::ComponentHandle;
use tokio::sync::mpsc::UnboundedSender;

/// What the player shows: the message playing or paused, and where it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlayerView {
    pub key: String,
    pub playing: bool,
    pub position_ms: i32,
    pub loading_key: String,
    pub speed: f32,
}

/// What the recorder bar shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecorderView {
    /// "idle" | "starting" | "recording" | "paused" | "sending"
    pub state: &'static str,
    pub elapsed_ms: i32,
    pub levels: Vec<f32>,
    pub preview_playing: bool,
    pub preview_ms: i32,
    pub problem: String,
}

impl UiHandle {
    pub fn voice_player(&self, v: PlayerView) {
        self.run(move |w: AppWindow| {
            let s = w.global::<VoiceStore>();
            s.set_key(v.key.into());
            s.set_playing(v.playing);
            s.set_position_ms(v.position_ms);
            s.set_loading_key(v.loading_key.into());
            s.set_speed(v.speed);
        });
    }

    pub fn voice_recorder(&self, v: RecorderView) {
        self.run(move |w: AppWindow| {
            let s = w.global::<VoiceRecStore>();
            s.set_state(v.state.into());
            s.set_elapsed_ms(v.elapsed_ms);
            s.set_levels(model(v.levels));
            s.set_preview_playing(v.preview_playing);
            s.set_preview_ms(v.preview_ms);
            s.set_problem(v.problem.into());
        });
    }
}

/// Connects the two stores' callbacks to the backend.
pub fn wire(window: &AppWindow, tx: UnboundedSender<Cmd>) {
    let send = move |c: VoiceCmd| {
        let _ = tx.send(Cmd::Voice(c));
    };
    let player = window.global::<VoiceStore>();
    {
        let send = send.clone();
        player.on_toggle(move |key, url| send(VoiceCmd::Toggle(key.into(), url.into())));
    }
    {
        let send = send.clone();
        player.on_seek(move |key, url, f| send(VoiceCmd::Seek(key.into(), url.into(), f)));
    }
    {
        let send = send.clone();
        player.on_cycle_speed(move || send(VoiceCmd::CycleSpeed));
    }
    let rec = window.global::<VoiceRecStore>();
    let on = |f: fn() -> VoiceCmd| {
        let send = send.clone();
        move || send(f())
    };
    rec.on_start(on(|| VoiceCmd::RecordStart));
    rec.on_pause(on(|| VoiceCmd::RecordPause));
    rec.on_resume(on(|| VoiceCmd::RecordResume));
    rec.on_discard(on(|| VoiceCmd::RecordDiscard));
    rec.on_send(on(|| VoiceCmd::RecordSend));
    rec.on_preview_toggle(on(|| VoiceCmd::PreviewToggle));
    {
        let send = send.clone();
        rec.on_preview_seek(move |f| send(VoiceCmd::PreviewSeek(f)));
    }
}
