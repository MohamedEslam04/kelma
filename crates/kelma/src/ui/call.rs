//! UI-thread side of the call window: fills the `CallStore` global (`ui/call_store.slint`).

use super::{UiHandle, model};
use crate::backend::call::{CallData, Found, Person, is_meeting_chat};
use crate::data::{Pixels, initials};
use crate::{AppWindow, CallCaption, CallFound, CallPerson, CallStore};
use slint::{ComponentHandle, Image, SharedString};
use std::cell::RefCell;
use std::collections::HashMap;

fn int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn texts(v: Vec<String>) -> slint::ModelRc<SharedString> {
    model(v.into_iter().map(SharedString::from).collect())
}

fn set(w: &AppWindow, d: CallData, photos: HashMap<String, Pixels>) {
    let s = w.global::<CallStore>();
    if d.phase.is_empty() || d.phase == "prejoin" {
        // A new call opens full size, with no panel or menu left from the last one.
        s.set_minimized(false);
        s.set_menu(SharedString::new());
        s.set_view_mode("gallery".into());
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
    s.set_transcribing(d.transcribing);
    s.set_captions(d.captions);
    s.set_caption_lines(model(
        d.caption_lines
            .into_iter()
            .map(|c| CallCaption {
                speaker: c.speaker.into(),
                text: c.text.into(),
            })
            .collect(),
    ));
    s.set_notice(d.notice.into());
    s.set_error(d.error.into());
    s.set_ended(d.ended.into());
    s.set_chat_id(d.chat_id.into());
    s.set_can_rejoin(d.can_rejoin);
    s.set_in_call(int(d.in_call));
    s.set_in_lobby(int(d.in_lobby));
    let images = photo_images(&photos);
    let person = |p| call_person(p, &images);
    let (mini, more) = mini_people(&d.people);
    s.set_mini_people(model(mini.into_iter().map(person).collect()));
    s.set_mini_more(int(more));
    s.set_speaker_people(model(
        speaker_order(&d.people).into_iter().map(person).collect(),
    ));
    s.set_people(model(d.people.into_iter().map(person).collect()));
    s.set_mics(texts(d.mics));
    s.set_mic(int(d.mic));
    s.set_speakers(texts(d.speakers));
    s.set_speaker(int(d.speaker));
    s.set_held(d.held);
    s.set_found(model(d.found.into_iter().map(call_found).collect()));
    s.set_invite_status(d.invite_status.into());
}

fn call_found(f: Found) -> CallFound {
    CallFound {
        id: f.mri.into(),
        initials: initials(&f.name).into(),
        name: f.name.into(),
        subtitle: f.subtitle.into(),
    }
}

thread_local! {
    /// The photos shown last, as the images made from them: the call state is replaced every
    /// second, and the same pixels should not be uploaded again each time.
    static PHOTO_IMAGES: RefCell<HashMap<String, (Pixels, Image)>> = RefCell::default();
}

/// The images for these photos, reusing the ones made from the same pixels before.
fn photo_images(photos: &HashMap<String, Pixels>) -> HashMap<String, Image> {
    PHOTO_IMAGES.with_borrow_mut(|cache| {
        // Only the people still in the call stay cached.
        cache.retain(|mri, _| photos.contains_key(mri));
        for (mri, px) in photos {
            let same = |old: &Pixels| std::ptr::eq(old.as_bytes().as_ptr(), px.as_bytes().as_ptr());
            if !cache.get(mri).is_some_and(|(old, _)| same(old)) {
                cache.insert(mri.clone(), (px.clone(), Image::from_rgba8(px.clone())));
            }
        }
        cache
            .iter()
            .map(|(mri, (_, img))| (mri.clone(), img.clone()))
            .collect()
    })
}

fn call_person(p: Person, images: &HashMap<String, Image>) -> CallPerson {
    let photo = images.get(&p.mri);
    CallPerson {
        has_photo: photo.is_some(),
        photo: photo.cloned().unwrap_or_default(),
        initials: initials(&p.name).into(),
        name: p.name.into(),
        muted: p.muted,
        hand: p.hand,
        lobby: p.lobby,
        me: p.me,
        role: p.role.into(),
        speaking: p.speaking,
    }
}

/// The floating call shows at most this many tiles.
const MINI_TILES: usize = 4;

/// Who the floating call shows: the others in the call, latest speakers first (the call's order
/// otherwise), or us when alone; and how many more there are beyond [`MINI_TILES`].
fn mini_people(people: &[Person]) -> (Vec<Person>, usize) {
    let mut others: Vec<&Person> = people.iter().filter(|p| !p.lobby && !p.me).collect();
    if others.is_empty() {
        others = people.iter().filter(|p| !p.lobby).collect();
    }
    // Stable: people who never spoke keep the roster's order.
    others.sort_by_key(|p| std::cmp::Reverse(p.spoke));
    let more = others.len().saturating_sub(MINI_TILES);
    (others.into_iter().take(MINI_TILES).cloned().collect(), more)
}

/// Who the speaker view shows, in-call people only: the latest speaker first (somebody else rather
/// than us until anyone speaks), then everyone else in the call's order.
fn speaker_order(people: &[Person]) -> Vec<Person> {
    let mut v: Vec<&Person> = people.iter().filter(|p| !p.lobby).collect();
    let lead = v
        .iter()
        .enumerate()
        .max_by_key(|(i, p)| (p.spoke, !p.me, std::cmp::Reverse(*i)))
        .map(|(i, _)| i);
    if let Some(i) = lead {
        let p = v.remove(i);
        v.insert(0, p);
    }
    v.into_iter().cloned().collect()
}

/// Wires what the call window answers by itself: whether a conversation is a meeting chat (the
/// header's Join button).
pub fn wire(window: &AppWindow) {
    window
        .global::<CallStore>()
        .on_is_meeting_chat(|id| is_meeting_chat(&id));
}

impl UiHandle {
    /// Replaces everything the call window shows; `photos` are the profile photos known by MRI.
    pub fn call_state(&self, d: CallData, photos: HashMap<String, Pixels>) {
        self.run(move |w| set(&w, d, photos));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, me: bool, lobby: bool, spoke: u64) -> Person {
        Person {
            name: name.into(),
            me,
            lobby,
            spoke,
            ..Default::default()
        }
    }

    fn names(v: &[Person]) -> Vec<&str> {
        v.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn the_floating_call_shows_the_latest_speakers_first_and_counts_the_rest() {
        let people = [
            p("Me", true, false, 9),
            p("Ada", false, false, 0),
            p("Bo", false, false, 3),
            p("Cy", false, true, 0),
            p("Di", false, false, 5),
            p("Ed", false, false, 0),
            p("Fa", false, false, 0),
        ];
        let (mini, more) = mini_people(&people);
        assert_eq!(names(&mini), ["Di", "Bo", "Ada", "Ed"]);
        assert_eq!(more, 1, "Fa; the lobby and we are not counted");
    }

    #[test]
    fn the_speaker_view_puts_the_latest_speaker_first_and_keeps_the_rest_in_order() {
        let people = [
            p("Me", true, false, 0),
            p("Ada", false, false, 3),
            p("Cy", false, true, 99),
            p("Bo", false, false, 7),
            p("Di", false, false, 0),
        ];
        assert_eq!(names(&speaker_order(&people)), ["Bo", "Me", "Ada", "Di"]);
    }

    #[test]
    fn the_speaker_view_leads_with_somebody_else_until_someone_speaks() {
        let people = [
            p("Me", true, false, 0),
            p("Ada", false, false, 0),
            p("Bo", false, false, 0),
        ];
        assert_eq!(names(&speaker_order(&people)), ["Ada", "Me", "Bo"]);
        assert_eq!(names(&speaker_order(&people[..1])), ["Me"]);
        assert!(speaker_order(&[]).is_empty());
    }

    #[test]
    fn we_lead_the_speaker_view_when_we_spoke_last() {
        let people = [p("Ada", false, false, 2), p("Me", true, false, 5)];
        assert_eq!(names(&speaker_order(&people)), ["Me", "Ada"]);
    }

    #[test]
    fn alone_the_floating_call_shows_us() {
        let (mini, more) = mini_people(&[p("Me", true, false, 0), p("Cy", false, true, 0)]);
        assert_eq!(names(&mini), ["Me"]);
        assert_eq!(more, 0);
    }
}
