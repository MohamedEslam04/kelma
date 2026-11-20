//! The call commands, run in the order they were sent.
//!
//! `Engine::dispatch` runs most commands each in its own task on a multi-thread runtime, so two
//! commands sent back to back (by one click, or two quick ones) run in either order: the post-call
//! survey's answer once lost the race to the close of the call-ended screen, and "mic off" then
//! "Join now" could join with the mic on. The call window, the 1:1 ringing flows and the Calls page
//! share the call state, so their commands go through one lane instead: a task that takes them from
//! a channel and runs them one by one. Every other store keeps its own tasks.
//!
//! The lane runs only the ordered part of a command: a plain function (not `async`, so it cannot
//! wait on the network) that changes the state and returns. What waits on the network (a people
//! search, loading the history, playing a voicemail) comes back as a [`Rest`] and runs in its own
//! task, so a slow search never holds up Mute or Leave.

use super::{Cmd, Engine};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// The part of a command that waits on the network, after its ordered part ran.
pub(super) type Rest = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

pub(super) fn rest(f: impl Future<Output = anyhow::Result<()>> + Send + 'static) -> Option<Rest> {
    Some(Box::pin(f))
}

/// Runs what is left of a command to its end (`Engine::handle`, which the tests drive one command
/// at a time).
pub(super) async fn finish(rest: Option<Rest>) -> anyhow::Result<()> {
    match rest {
        Some(r) => r.await,
        None => Ok(()),
    }
}

impl Engine {
    /// Queues a call command on the lane, which starts with the first one.
    pub(super) fn in_order(self: &Arc<Self>, cmd: Cmd) {
        let lane = self.calls_lane.get_or_init(|| {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let engine = Arc::downgrade(self);
            tokio::spawn(async move {
                while let Some(cmd) = rx.recv().await {
                    let Some(e) = engine.upgrade() else { break };
                    e.run_in_order(cmd);
                }
            });
            tx
        });
        let _ = lane.send(cmd);
    }

    fn run_in_order(self: &Arc<Self>, cmd: Cmd) {
        let ordered = match cmd {
            Cmd::Call(c) => self.call_cmd(c),
            #[cfg(feature = "calls")]
            Cmd::DirectCall(c) => self.direct_call_cmd(c).map(|()| None),
            #[cfg(feature = "calls")]
            Cmd::CallsPage(c) => self.calls_page_cmd(c),
            cmd => {
                tokio::spawn(self.clone().handle(cmd));
                return;
            }
        };
        match ordered {
            Ok(None) => {}
            Ok(Some(rest)) => {
                let e = self.clone();
                tokio::spawn(async move {
                    if let Err(err) = rest.await {
                        e.command_failed(err);
                    }
                });
            }
            Err(err) => self.command_failed(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Cmd, Engine};
    use crate::backend::call::{CallCmd, FakeDriver};
    use crate::backend::calls_page::CallsCmd;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    const THREAD: &str = "19:meeting_abc@thread.v2";

    /// The offline engine and its window (kept alive for the engine's UI handle).
    fn engine() -> (Arc<Engine>, crate::AppWindow) {
        i_slint_backend_testing::init_no_event_loop();
        let window = crate::AppWindow::new().unwrap();
        let ui = crate::ui::UiHandle::new(slint::ComponentHandle::as_weak(&window));
        let e = Engine::new(
            teams_proto::Client::offline("test").unwrap(),
            store::Store::open_in_memory().unwrap(),
            session::Config::default(),
            ui,
            "8:me".into(),
            "Me".into(),
        );
        e.offline.store(true, Ordering::Relaxed);
        (e, window)
    }

    /// The live app's runtime (`main.rs`): two workers.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    /// Sends `cmds` as the live app does: one after another from the command loop's task, each
    /// through `Engine::dispatch`.
    fn send(rt: &tokio::runtime::Runtime, e: &Arc<Engine>, cmds: Vec<Cmd>) {
        let e = e.clone();
        rt.block_on(rt.spawn(async move {
            for c in cmds {
                e.dispatch(c);
            }
        }))
        .unwrap();
    }

    /// Waits (up to 5 s) for `done`, then a little longer for anything still running.
    fn settle(done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn call_window_commands_run_in_the_order_they_were_sent() {
        let rt = runtime();
        let (e, _window) = engine();
        let fake = FakeDriver::manual();
        e.calls.set_driver(Arc::new(fake.clone()));
        let mut cmds = vec![
            Cmd::Call(CallCmd::JoinChat(THREAD.into())),
            Cmd::Call(CallCmd::PrejoinMic(false)),
            Cmd::Call(CallCmd::JoinNow),
        ];
        let mut want = vec![format!("join chat:{THREAD} mic=off")];
        for i in 0..40 {
            let start = i % 2 == 0;
            cmds.push(Cmd::Call(CallCmd::Recording(start)));
            want.push(format!(
                "recording {}",
                if start { "start" } else { "stop" }
            ));
        }
        cmds.push(Cmd::Call(CallCmd::Leave));
        want.push("leave".into());

        send(&rt, &e, cmds);
        settle(|| fake.calls().iter().any(|l| l == "leave"));
        assert_eq!(fake.calls(), want);
    }

    #[test]
    fn calls_page_commands_run_in_the_order_they_were_sent() {
        let rt = runtime();
        let (e, _window) = engine();
        let (a, b, c) = ("8:orgid:a", "8:orgid:b", "8:orgid:c");
        let mut cmds = Vec::new();
        // Each round: the three join the speed dial, `c` moves to the top, then all leave again.
        for _ in 0..10 {
            for m in [a, b, c] {
                cmds.push(Cmd::CallsPage(CallsCmd::SpeedToggle(m.into())));
            }
            cmds.push(Cmd::CallsPage(CallsCmd::SpeedMove(c.into(), -1)));
            cmds.push(Cmd::CallsPage(CallsCmd::SpeedMove(c.into(), -1)));
            for m in [a, b, c] {
                cmds.push(Cmd::CallsPage(CallsCmd::SpeedToggle(m.into())));
            }
        }
        for m in [a, b, c] {
            cmds.push(Cmd::CallsPage(CallsCmd::SpeedToggle(m.into())));
        }
        cmds.push(Cmd::CallsPage(CallsCmd::SpeedMove(c.into(), -1)));
        cmds.push(Cmd::CallsPage(CallsCmd::SpeedMove(c.into(), -1)));

        send(&rt, &e, cmds);
        let order = || -> Vec<String> {
            e.config()
                .speed_dial
                .iter()
                .map(|s| s.mri.clone())
                .collect()
        };
        settle(|| order() == [c, a, b]);
        assert_eq!(order(), [c, a, b]);
    }
}
