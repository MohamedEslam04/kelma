//! People's presence for the dots on their avatars (chat plan 6.4).
//!
//! The chat list, the message list (each author's photo), the chat header and the people lists
//! ask for the people they draw ([`want`]). Someone asked for the first time is looked up soon
//! after (one batch for everyone new, so opening a chat costs one request); everyone is looked up
//! again once a minute in batches, and the presence pushes from trouter apply at once. A change
//! redraws the chat list, the messages and the header. Our own availability goes to the photo at
//! the top right.

use super::Engine;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Cache {
    known: HashMap<String, String>,
    wanted: HashSet<String>,
    /// Wanted since the last lookup: looked up without waiting for the next round.
    fresh: HashSet<String>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
/// Wakes the lookup loop when someone new is wanted.
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();
/// Everyone wanted is looked up again this often.
const REFRESH: Duration = Duration::from_secs(60);
/// After a wake, how long the loop waits for the rest of the people a screen asks for.
const GATHER: Duration = Duration::from_millis(300);

fn with<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    let mut g = CACHE.lock().unwrap();
    f(g.get_or_insert_with(Cache::default))
}

/// The person's availability as last seen (`""` when unknown), and asks for it from now on.
pub fn want(mri: &str) -> String {
    let (known, new) = with(|c| {
        let new = c.wanted.insert(mri.to_string());
        if new {
            c.fresh.insert(mri.to_string());
        }
        (c.known.get(mri).cloned().unwrap_or_default(), new)
    });
    if new {
        WAKE.notify_one();
    }
    known
}

/// The person's availability as last seen, without asking for it.
pub fn known(mri: &str) -> String {
    with(|c| c.known.get(mri).cloned().unwrap_or_default())
}

/// Records an availability; true when it changed.
pub fn set(mri: &str, availability: &str) -> bool {
    with(|c| {
        let old = c.known.insert(mri.to_string(), availability.to_string());
        old.as_deref() != Some(availability)
    })
}

/// An availability in words, as Teams shows it (`""` for unknown values).
pub fn label(availability: &str) -> &'static str {
    match availability.strip_suffix("Idle").unwrap_or(availability) {
        "Available" => "Available",
        "Busy" => "Busy",
        "DoNotDisturb" => "Do not disturb",
        "BeRightBack" => "Be right back",
        "Away" => "Away",
        "Offline" => "Offline",
        _ => "",
    }
}

/// A work location pin in words.
pub fn work_location_label(pin: Option<&str>) -> &'static str {
    let Some(pin) = pin else { return "" };
    match ::calendar::WorkLocation::from_value(&serde_json::json!(pin)) {
        ::calendar::WorkLocation::Office => "In the office today",
        ::calendar::WorkLocation::Remote => "Working remotely today",
        ::calendar::WorkLocation::Unset => "",
    }
}

/// Who to look up: everyone wanted on a full round, else only the people wanted since the last.
fn take(all: bool) -> Vec<String> {
    with(|c| {
        let fresh = std::mem::take(&mut c.fresh);
        if all {
            c.wanted.iter().cloned().collect()
        } else {
            fresh.into_iter().collect()
        }
    })
}

impl Engine {
    /// Looks up the wanted people's presence: the new ones soon after they are wanted, everyone
    /// once a minute (live only).
    pub(super) fn start_people_presence(self: &Arc<Self>) {
        let e = self.clone();
        want(&e.me);
        tokio::spawn(async move {
            loop {
                let all = tokio::select! {
                    _ = WAKE.notified() => {
                        tokio::time::sleep(GATHER).await;
                        false
                    }
                    _ = tokio::time::sleep(REFRESH) => true,
                };
                let people = take(all);
                if people.is_empty() {
                    continue;
                }
                match e.client.get_presence(&people).await {
                    Ok(found) => {
                        for p in found {
                            e.apply_presence(&p.mri, &p.availability);
                        }
                    }
                    Err(err) => tracing::debug!(%err, "people presence not read"),
                }
                // Also after the profile menu showed an older read of our own status.
                let mine = known(&e.me);
                if !mine.is_empty() {
                    e.ui.me_presence(mine);
                }
            }
        });
    }

    /// A presence push from trouter.
    pub(super) fn on_presence(self: &Arc<Self>, mri: &str, availability: &str) {
        self.apply_presence(mri, availability);
    }

    /// Records an availability; a change redraws the dots that show it.
    pub(super) fn apply_presence(self: &Arc<Self>, mri: &str, availability: &str) {
        if !set(mri, availability) {
            return;
        }
        if mri == self.me {
            self.ui.me_presence(availability.to_string());
        }
        self.mark(|d| {
            d.chats = true;
            d.messages = true;
            d.header = true;
        });
    }
}

#[cfg(test)]
impl Engine {
    /// Records an availability as if it had been read (no network in tests).
    pub(crate) fn test_set_presence(self: &Arc<Self>, mri: &str, availability: &str) {
        self.apply_presence(mri, availability);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_and_location_in_words() {
        assert_eq!(label("DoNotDisturb"), "Do not disturb");
        assert_eq!(label("BusyIdle"), "Busy");
        assert_eq!(label("PresenceUnknown"), "");
        assert_eq!(work_location_label(Some("office")), "In the office today");
        assert_eq!(work_location_label(Some("2")), "Working remotely today");
        assert_eq!(work_location_label(None), "");
    }

    #[test]
    fn wanted_people_are_remembered_and_changes_reported() {
        assert_eq!(want("8:orgid:pp-test-a"), "");
        assert!(with(|c| c.wanted.contains("8:orgid:pp-test-a")));
        assert!(set("8:orgid:pp-test-a", "Busy"));
        assert!(!set("8:orgid:pp-test-a", "Busy"));
        assert_eq!(want("8:orgid:pp-test-a"), "Busy");
        assert!(set("8:orgid:pp-test-a", "Available"));
    }

    #[test]
    fn people_wanted_for_the_first_time_are_looked_up_in_one_batch() {
        want("8:orgid:pp-test-fresh-1");
        want("8:orgid:pp-test-fresh-2");
        want("8:orgid:pp-test-fresh-1");
        // Other tests share the cache: only this test's people count.
        let mine = |v: Vec<String>| {
            let mut v: Vec<String> = v
                .into_iter()
                .filter(|m| m.starts_with("8:orgid:pp-test-fresh"))
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            mine(take(false)),
            ["8:orgid:pp-test-fresh-1", "8:orgid:pp-test-fresh-2"]
        );
        assert!(mine(take(false)).is_empty(), "asked once");
        assert_eq!(mine(take(true)).len(), 2, "but kept for the rounds");
    }
}
