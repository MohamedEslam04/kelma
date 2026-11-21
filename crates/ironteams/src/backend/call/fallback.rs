//! The safety net of a meeting join that declares its video lines: when the media servers refuse
//! them, IronTeams rejoins the same meeting audio only, says so, and remembers it for the rest of
//! the session.
//!
//! Live 2026-11-20 (`docs/protocol-calls.md`, Remote video): a join that declared its video lines
//! was taken (201) and ended within 1.5 s by a `conversationEnd` and `callEnd` of 410, subCode
//! 301999, "InvalidFinalAnswerMediaDescriptions"; an `UpdateMediaDescriptions` naming lines the
//! join had not declared was refused (400, 10146, "Invalid MediaDescriptions") and the call ended
//! 0.2 s later with 410, 301999, "InvalidUpdatedMediaDescriptions".

use calling::invitation::EndReason;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A call that ends with the media-description subCode this soon after joining is taken as the
/// servers refusing the video lines, even without the phrase.
pub const EARLY: Duration = Duration::from_secs(20);

/// What the call says after the rejoin.
pub const NOTE: &str = "Video isn't working in this meeting yet, so you joined with audio.";

/// The `subCode` of the 410 that followed both refusals (live 2026-11-20).
const MEDIA_DESCRIPTIONS_SUB_CODE: i64 = 301999;

fn names_media_descriptions(text: &str) -> bool {
    text.to_ascii_lowercase().contains("mediadescriptions")
}

/// Whether the service ending (or rejecting) the call is it refusing our video lines, so the join
/// is redone audio only. `had_video_lines` is whether this join declared any: the audio-only
/// rejoin declares none, so a second refusal never loops. The phrase counts at any time (a
/// sharing update refused minutes in); the bare subCode only in the first [`EARLY`] seconds.
pub fn call_refused_video(had_video_lines: bool, reason: &EndReason, since_join: Duration) -> bool {
    had_video_lines
        && (names_media_descriptions(&reason.phrase)
            || (reason.sub_code == MEDIA_DESCRIPTIONS_SUB_CODE && since_join <= EARLY))
}

/// Whether a failed join (the error as text, the service's body included) is it refusing our
/// video lines.
pub fn join_refused_video(had_video_lines: bool, error: &str) -> bool {
    had_video_lines && names_media_descriptions(error)
}

/// Whether a refusal was seen in this session: later meeting joins go straight to audio only.
#[derive(Debug, Default)]
pub struct Memory(AtomicBool);

impl Memory {
    pub fn video_allowed(&self) -> bool {
        !self.0.load(Ordering::SeqCst)
    }

    pub fn remember_refusal(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(sub_code: i64, phrase: &str) -> EndReason {
        EndReason {
            code: 410,
            sub_code,
            phrase: phrase.into(),
        }
    }

    const SOON: Duration = Duration::from_secs(2);
    const LATE: Duration = Duration::from_secs(300);

    #[test]
    fn the_refusals_seen_live_trigger_the_fallback() {
        let join = end(301999, "InvalidFinalAnswerMediaDescriptions");
        let update = end(301999, "InvalidUpdatedMediaDescriptions");
        assert!(call_refused_video(true, &join, SOON));
        assert!(call_refused_video(true, &update, SOON));
        assert!(join_refused_video(
            true,
            "join_meeting: HTTP 400: Invalid MediaDescriptions"
        ));
    }

    #[test]
    fn the_phrase_counts_late_but_the_bare_subcode_only_early() {
        let phrase = end(1, "InvalidUpdatedMediaDescriptions");
        assert!(call_refused_video(true, &phrase, LATE));
        let bare = end(301999, "");
        assert!(call_refused_video(true, &bare, SOON));
        assert!(call_refused_video(true, &bare, EARLY));
        assert!(!call_refused_video(true, &bare, LATE));
    }

    #[test]
    fn other_endings_and_errors_do_not() {
        assert!(!call_refused_video(true, &end(410, "Normal"), SOON));
        assert!(!call_refused_video(true, &EndReason::default(), SOON));
        assert!(!join_refused_video(
            true,
            "join_meeting: HTTP 403: Forbidden"
        ));
        assert!(!join_refused_video(true, "network down"));
    }

    #[test]
    fn it_happens_only_once() {
        // The audio-only rejoin declares no lines, so a refusal there ends the call for real.
        let join = end(301999, "InvalidFinalAnswerMediaDescriptions");
        assert!(!call_refused_video(false, &join, SOON));
        assert!(!join_refused_video(
            false,
            "HTTP 400: Invalid MediaDescriptions"
        ));
    }

    #[test]
    fn a_refusal_is_remembered_for_the_session() {
        let session = Memory::default();
        assert!(session.video_allowed());
        session.remember_refusal();
        assert!(!session.video_allowed());
        // Another session (a new sign-in builds a new hub) starts clean.
        assert!(Memory::default().video_allowed());
    }
}
