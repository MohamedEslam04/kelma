//! This device's presence while IronTeams runs (chat plan 6.4).
//!
//! The presence service only shows a status the user forced, and "Available", while one of the
//! user's devices is registered and in use. So IronTeams registers itself at sign-in (Available,
//! reported by its connection) and checks once a minute whether the screen is locked: while it is,
//! IronTeams removes its device (the user then shows as their other devices make them, usually Away);
//! when it is unlocked again, IronTeams registers again. A call keeps its "In a call" presence
//! ([`super::call_presence`]) whatever the screen does.

use super::Engine;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use teams_proto::Client;
use teams_proto::presence::Availability;

/// Whether the loop runs (so quitting has a device to remove).
static STARTED: AtomicBool = AtomicBool::new(false);

/// How often the screen lock is checked.
const CHECK: Duration = Duration::from_secs(60);

/// What to do after a check, given whether the device is registered now.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    Register,
    Remove,
    Nothing,
}

fn step(registered: bool, locked: bool, in_call: bool) -> Step {
    match (registered, locked && !in_call) {
        (false, false) => Step::Register,
        (true, true) => Step::Remove,
        _ => Step::Nothing,
    }
}

impl Engine {
    /// Starts the presence loop (live only; demo mode and tests never call it).
    pub(super) fn start_my_presence(self: &Arc<Self>) {
        let client = self.client.clone();
        STARTED.store(true, Ordering::SeqCst);
        tokio::spawn(async move { run(client).await });
    }

    /// IronTeams is closing: removes this device from presence, in the moment main waits before the
    /// runtime shuts down.
    pub(super) fn stop_my_presence(self: &Arc<Self>) {
        if !STARTED.swap(false, Ordering::SeqCst) {
            return;
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ =
                tokio::time::timeout(Duration::from_millis(500), client.remove_endpoint()).await;
        });
    }
}

async fn run(client: Client) {
    let mut registered = false;
    loop {
        let locked = tray::screen::screen_locked().await.unwrap_or(false);
        match step(registered, locked, super::call_presence::in_call()) {
            Step::Register => {
                match client
                    .publish_endpoint(Availability::Available, "Available")
                    .await
                {
                    Ok(()) => registered = true,
                    Err(e) => tracing::debug!(%e, "presence: could not register this device"),
                }
            }
            Step::Remove => {
                if let Err(e) = client.remove_endpoint().await {
                    tracing::debug!(%e, "presence: could not remove this device");
                }
                registered = false;
            }
            Step::Nothing => {}
        }
        tokio::time::sleep(CHECK).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_while_in_use_and_steps_back_while_locked() {
        assert_eq!(step(false, false, false), Step::Register);
        assert_eq!(step(true, false, false), Step::Nothing);
        assert_eq!(step(true, true, false), Step::Remove);
        assert_eq!(step(false, true, false), Step::Nothing);
        // A call keeps the device registered even with the screen locked.
        assert_eq!(step(true, true, true), Step::Nothing);
    }
}
