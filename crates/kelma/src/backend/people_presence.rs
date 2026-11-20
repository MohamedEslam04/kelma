//! Other people's presence for the dots on their avatars (chat plan 6.4).
//!
//! The chat list asks for the people it draws ([`want`]); the engine looks them up once a minute
//! in batches and also applies the presence pushes from trouter. A change marks the chat list for
//! redrawing.

use super::Engine;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Cache {
    known: HashMap<String, String>,
    wanted: HashSet<String>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    let mut g = CACHE.lock().unwrap();
    f(g.get_or_insert_with(Cache::default))
}

/// The person's availability as last seen (`""` when unknown), and asks for it from now on.
pub fn want(mri: &str) -> String {
    with(|c| {
        c.wanted.insert(mri.to_string());
        c.known.get(mri).cloned().unwrap_or_default()
    })
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

fn wanted() -> Vec<String> {
    with(|c| c.wanted.iter().cloned().collect())
}

impl Engine {
    /// Looks up the wanted people's presence once a minute (live only).
    pub(super) fn start_people_presence(self: &Arc<Self>) {
        let e = self.clone();
        tokio::spawn(async move {
            loop {
                // The first chat list draws first, so there is a list to ask for.
                tokio::time::sleep(Duration::from_secs(5)).await;
                let people = wanted();
                if !people.is_empty() {
                    match e.client.get_presence(&people).await {
                        Ok(found) => {
                            let mut changed = false;
                            for p in found {
                                changed |= set(&p.mri, &p.availability);
                            }
                            if changed {
                                e.mark(|d| d.chats = true);
                            }
                        }
                        Err(err) => tracing::debug!(%err, "people presence not read"),
                    }
                }
                tokio::time::sleep(Duration::from_secs(55)).await;
            }
        });
    }

    /// A presence push from trouter.
    pub(super) fn on_presence(self: &Arc<Self>, mri: &str, availability: &str) {
        if set(mri, availability) {
            self.mark(|d| d.chats = true);
        }
    }
}

#[cfg(test)]
impl Engine {
    /// Records an availability as if it had been read (no network in tests).
    pub(crate) fn test_set_presence(&self, mri: &str, availability: &str) {
        set(mri, availability);
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
        assert!(wanted().contains(&"8:orgid:pp-test-a".to_string()));
        assert!(set("8:orgid:pp-test-a", "Busy"));
        assert!(!set("8:orgid:pp-test-a", "Busy"));
        assert_eq!(want("8:orgid:pp-test-a"), "Busy");
        assert!(set("8:orgid:pp-test-a", "Available"));
    }
}
