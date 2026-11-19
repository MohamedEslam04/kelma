//! Operations on a running 1:1 call: add a person, hold and resume, and blind transfer.
//!
//! - **Add a person**: `POST addParticipant` (the conversation's link, else
//!   `<controller>/addParticipant`) with the person in `participants.to`, a fresh
//!   `participantId`, and our `addParticipantSuccess`/`addParticipantFailure` callbacks. Without a
//!   group chat thread the web client posts to `/addParticipant` with `debugContent: {}` in place of
//!   `groupContext`/`groupChat` (mautrix-teams `calls.go` `AddParticipant` and `teams-webclient.yaml`
//!   `/add`, AGPL, facts only; `displayName` from rusty-teams `organizer.rs`
//!   `add_participants_body`, MIT). UNVERIFIED live: both read it from the web client's code, and
//!   rusty-teams escalates a 1:1 call through a new group chat (`addParticipantAndModality`)
//!   instead. The service then moves the call to a conference; its re-offer is answered like the
//!   lobby's.
//! - **Hold / resume**: a re-offer on the same peer connection through the leg's
//!   `mediaRenegotiation` link (`newOffer: false`), with every media line `a=inactive` to hold and
//!   as the session offers it to resume. The answer is applied with our own directions mirrored
//!   back ([`keep_media_running`]), so the local media never stops; the caller silences it
//!   instead. Facts: rusty-teams `hold.rs` `all_inactive`/`keep_media_running` and
//!   `call/peer_sdp.rs` `offer_same_peer` (MIT). The acceptance acknowledgement also names a
//!   `hold` link; no reference posts to it, and Teams on the web showed only a media answer
//!   acknowledgement on resume. UNVERIFIED live.
//! - **Transfer now** (blind): `POST transfer` (the leg's link from the acceptance acknowledgement)
//!   with `callTransfer {target, transferor, links {transferAcceptance, transferCompletion}}`;
//!   `call/transferCompletion` with code 0 (or 2xx) says it went through, and we leave. Facts:
//!   rusty-teams `transfer.rs` `transfer_body`/`completion` (MIT). UNVERIFIED live.

use super::controls::control_url;
use super::CallSession;
use crate::types::Links;
use serde_json::{json, Value};

const DIRECTIONS: [&str; 4] = ["sendrecv", "sendonly", "recvonly", "inactive"];
const INACTIVE: &str = "inactive";

/// `POST addParticipant`: ring `mri` into the call.
pub fn add_participant_body(s: &CallSession, mri: &str, display_name: &str) -> Value {
    json!({
        "disableUnmute": false,
        "participants": {
            "from": s.me.wire(),
            "to": [{
                "id": mri,
                "displayName": display_name,
                "participantId": uuid::Uuid::new_v4().to_string(),
            }],
        },
        "participantInvitationData": {},
        "replacementDetails": null,
        "debugContent": {},
        "links": s.callbacks.conversation_links(&["addParticipantSuccess", "addParticipantFailure"]),
    })
}

/// Where and what to post to add `mri`: the `addParticipant` link, else under the controller.
pub fn add_participant_request(
    s: &CallSession,
    links: &Links,
    controller: &str,
    mri: &str,
    display_name: &str,
) -> (String, Value) {
    (
        control_url(links, controller, "addParticipant"),
        add_participant_body(s, mri, display_name),
    )
}

/// `POST transfer`: hand the other person over to `target` (an MRI) and drop out.
pub fn transfer_body(s: &CallSession, target: &str) -> Value {
    let me = &s.me;
    json!({
        "callTransfer": {
            "target": {"id": target},
            "transferor": {
                "details": {
                    "id": me.mri,
                    "endpointId": me.endpoint_id,
                    "participantId": me.participant_id,
                    "languageId": me.language_id,
                },
                "authorizationToken": null,
            },
            "links": s.callbacks.call_links(&["transferAcceptance", "transferCompletion"]),
            "replacementDetails": null,
            "disableForwardingAndUnanswered": false,
            "transferContext": null,
        },
    })
}

/// The transfer request on the leg's `transfer` link; `None` when the call has none.
pub fn transfer_request(s: &CallSession, links: &Links, target: &str) -> Option<(String, Value)> {
    links
        .get("transfer")
        .map(|url| (url.to_owned(), transfer_body(s, target)))
}

/// How a transfer ended (`call/transferCompletion`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferOutcome {
    Done,
    Failed { code: i64, sub_code: i64 },
}

/// Reads a `call/transferCompletion` body; `None` when it is not one.
pub fn parse_transfer_completion(body: &Value) -> Option<TransferOutcome> {
    let c = body.get("transferCompletion")?;
    let code = c["code"].as_i64().unwrap_or_default();
    Some(if code == 0 || (200..300).contains(&code) {
        TransferOutcome::Done
    } else {
        TransferOutcome::Failed {
            code,
            sub_code: c["subCode"].as_i64().unwrap_or_default(),
        }
    })
}

/// One media section of an SDP: its mid and the line index of its direction attribute.
struct Section {
    mid: String,
    direction: Option<usize>,
}

fn split(sdp: &str) -> (Vec<String>, &'static str) {
    let newline = if sdp.contains("\r\n") { "\r\n" } else { "\n" };
    let lines = sdp
        .split('\n')
        .map(|l| l.trim_end_matches('\r').to_owned())
        .collect();
    (lines, newline)
}

fn sections(lines: &[String]) -> Vec<Section> {
    let mut found: Vec<Section> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("m=") {
            found.push(Section {
                mid: String::new(),
                direction: None,
            });
        } else if let Some(s) = found.last_mut() {
            if let Some(mid) = line.strip_prefix("a=mid:") {
                s.mid = mid.to_owned();
            } else if line
                .strip_prefix("a=")
                .is_some_and(|d| DIRECTIONS.contains(&d))
            {
                s.direction = Some(i);
            }
        }
    }
    found
}

/// `sdp` with the direction of every media section that has one set by `pick(mid)`.
fn rewrite(sdp: &str, pick: impl Fn(&str) -> Option<String>) -> String {
    let (mut lines, newline) = split(sdp);
    for s in sections(&lines) {
        if let (Some(i), Some(d)) = (s.direction, pick(&s.mid)) {
            lines[i] = format!("a={d}");
        }
    }
    lines.join(newline)
}

/// Our offer as a hold: every media section `a=inactive`, nothing else changed.
pub fn hold_offer(sdp: &str) -> String {
    rewrite(sdp, |_| Some(INACTIVE.to_owned()))
}

/// The answer to a hold offer, with each section's direction mirrored from `our_offer` as the
/// session made it (before [`hold_offer`]), so applying it stops nothing locally.
pub fn keep_media_running(answer: &str, our_offer: &str) -> String {
    let (lines, _) = split(our_offer);
    let offered: Vec<(String, String)> = sections(&lines)
        .into_iter()
        .filter_map(|s| {
            let d = lines[s.direction?].strip_prefix("a=")?;
            let mirrored = match d {
                "sendonly" => "recvonly",
                "recvonly" => "sendonly",
                "inactive" => INACTIVE,
                _ => "sendrecv",
            };
            Some((s.mid, mirrored.to_owned()))
        })
        .collect();
    rewrite(answer, |mid| {
        offered
            .iter()
            .find(|(m, _)| m == mid)
            .map(|(_, d)| d.clone())
    })
}
