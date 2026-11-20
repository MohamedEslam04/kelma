//! Admitting people from the meeting's lobby (an organizer or presenter's operation).
//!
//! `POST admit` (the conversation's link, else `<controller>/admit`) with the person in
//! `participants.to`, our `admitSuccess`-style callbacks and a fresh `debugContent.causeId`. The
//! answer is a 202; the outcome comes on `conversation/admitParticipantSuccess/` (then a roster
//! delta with the person under `call`) or `admitParticipantFailure/`. Facts: rusty-teams
//! `organizer.rs` `admit_body` and `signaling.rs` `admit` (MIT, exercised live there); mautrix-teams
//! `calls.go` `decideLobby` (AGPL, facts only), which names the callbacks `admitSuccess` and
//! `admitFailure` where rusty-teams names them `admitParticipantSuccess`/`admitParticipantFailure`.
//! UNVERIFIED live in kelma: the callback names follow rusty-teams.
//!
//! **Admit all** is one `admit` per person waiting. The conversation also has an `admitAll` link,
//! but its body is not captured anywhere, and rusty-teams found that the service's `admitAll`
//! leaves anonymous guests waiting while listing them admits everyone.

use super::CallSession;
use super::controls::control_url;
use crate::types::Links;
use serde_json::{Value, json};

/// `POST admit`: let `mri` (shown as `display_name`) in from the lobby. `cause_id` is a fresh
/// UUID per request ([`admit_request`] makes it).
pub fn admit_body(s: &CallSession, mri: &str, display_name: &str, cause_id: &str) -> Value {
    json!({
        "participants": {
            "from": s.me.wire(),
            "to": [{"id": mri, "displayName": display_name}],
        },
        "links": {
            "admitFailure": s.callbacks.conversation("admitParticipantFailure"),
            "admitSuccess": s.callbacks.conversation("admitParticipantSuccess"),
        },
        "debugContent": {"causeId": cause_id},
    })
}

/// Where and what to post to admit `mri`: the `admit` link, else `<controller>/admit`.
pub fn admit_request(
    s: &CallSession,
    links: &Links,
    controller: &str,
    mri: &str,
    display_name: &str,
) -> (String, Value) {
    let cause = uuid::Uuid::new_v4().to_string();
    (
        control_url(links, controller, "admit"),
        admit_body(s, mri, display_name, &cause),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signaling::{CallbackLinks, LocalEndpoint, insert_path};

    const CONTROLLER: &str =
        "https://api.flightproxy.skype.com/api/v2/cp/conv-example.conv.skype.com/conv/AAAA?i=1";
    const GUEST: &str = "8:orgid:00000000-0000-0000-0000-0000000000c3";

    fn session() -> CallSession {
        CallSession {
            me: LocalEndpoint {
                mri: "8:orgid:00000000-0000-0000-0000-0000000000b2".into(),
                display_name: "Self Example".into(),
                endpoint_id: "sess-1".into(),
                participant_id: "part-1".into(),
                language_id: "en-US".into(),
            },
            callbacks: CallbackLinks::new("https://trouter.example.invalid/v4/f/AbCd/", "sess-1"),
            chain_id: "chain-1".into(),
            media_leg_id: "LEG1".into(),
        }
    }

    #[test]
    fn admitting_names_the_person_us_and_both_callbacks() {
        let s = session();
        let b = admit_body(&s, GUEST, "Guest Example", "cause-1");
        assert_eq!(b["participants"]["from"], s.me.wire());
        assert_eq!(
            b["participants"]["to"],
            json!([{"id": GUEST, "displayName": "Guest Example"}])
        );
        assert_eq!(
            b["links"]["admitSuccess"],
            s.callbacks.conversation("admitParticipantSuccess")
        );
        assert_eq!(
            b["links"]["admitFailure"],
            s.callbacks.conversation("admitParticipantFailure")
        );
        assert!(
            b["links"]["admitSuccess"]
                .as_str()
                .unwrap()
                .contains("/conversation/admitParticipantSuccess/")
        );
        assert_eq!(b["debugContent"]["causeId"], "cause-1");
    }

    #[test]
    fn the_admit_goes_to_its_link_else_under_the_controller_with_a_fresh_cause() {
        let s = session();
        let mut links = Links::default();
        let (url, a) = admit_request(&s, &links, CONTROLLER, GUEST, "Guest Example");
        assert_eq!(url, insert_path(CONTROLLER, "/admit"));
        let (_, b) = admit_request(&s, &links, CONTROLLER, GUEST, "Guest Example");
        assert_ne!(
            a["debugContent"]["causeId"], b["debugContent"]["causeId"],
            "each admit is a new operation"
        );
        links
            .0
            .insert("admit".into(), "https://conv.example.invalid/admit".into());
        let (url, _) = admit_request(&s, &links, CONTROLLER, GUEST, "Guest Example");
        assert_eq!(url, "https://conv.example.invalid/admit");
    }
}
