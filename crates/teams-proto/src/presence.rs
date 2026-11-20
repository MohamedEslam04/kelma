//! Presence: read people's availability, set ours, and publish a status note.
//!
//! The regional unified presence service named in the sign-in answer (`regionGtms.presenceUPS`,
//! e.g. `teams.cloud.microsoft/ups/noam/v1`, as Teams web uses it), endpoints from
//! `docs/research/open-source.md` §3.1 (rusty, teams-lite, better-teams, purple-teams). Requests
//! carry the presence-audience bearer token ([`Audience::Presence`]) and the `x-ms-*` client
//! headers the service insists on. Push updates arrive on trouter and are already parsed into
//! [`crate::model::Event::Presence`].
//!
//! Live status (2026-11-19, one work tenant): the global `presence.teams.microsoft.com` host
//! answers 401 to this token; the regional service answers `getpresence` with 200.

use crate::broker::Audience;
use crate::client::Client;
use crate::client_info::CLIENT_VERSION;
use crate::error::{Error, Result};
use reqwest::Method;
use serde_json::{Value, json};

/// Presence lookups per request. Teams web uses 200; the service accepts more.
const LOOKUP_CHUNK: usize = 200;
/// Expiry Teams sends for a status note that never expires.
pub const NEVER_EXPIRES: &str = "9999-12-31T23:59:59.999Z";
/// Suffix on a status note's message that means "show when people message me".
const PINNED_NOTE: &str = "<pinnednote></pinnednote>";

/// Availability values Teams accepts when forcing a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Available,
    Busy,
    DoNotDisturb,
    BeRightBack,
    Away,
    Offline,
}

impl Availability {
    /// The value on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Availability::Available => "Available",
            Availability::Busy => "Busy",
            Availability::DoNotDisturb => "DoNotDisturb",
            Availability::BeRightBack => "BeRightBack",
            Availability::Away => "Away",
            Availability::Offline => "Offline",
        }
    }

    /// Maps a reported availability (including `…Idle` variants) to the closest value.
    pub fn from_wire(s: &str) -> Option<Availability> {
        Some(match s.strip_suffix("Idle").unwrap_or(s) {
            "Available" => Availability::Available,
            "Busy" => Availability::Busy,
            "DoNotDisturb" => Availability::DoNotDisturb,
            "BeRightBack" => Availability::BeRightBack,
            "Away" => Availability::Away,
            "Offline" => Availability::Offline,
            _ => return None,
        })
    }
}

/// A status note ("Set status message").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusNote {
    pub text: String,
    pub expiry: Option<String>,
    /// "Show when people message me".
    pub show_when_messaged: bool,
}

/// One person's presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub mri: String,
    /// As reported: `Available`, `Busy`, `Away`, `BusyIdle`, `PresenceUnknown`, …
    pub availability: String,
    /// As reported: `InACall`, `InAMeeting`, `OffWork`, …
    pub activity: String,
    /// Out of office, from the calendar.
    pub ooo: bool,
    pub note: Option<StatusNote>,
    /// Work location when shared, as reported.
    pub work_location: Option<String>,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Body for `getpresence`.
pub fn lookup_body(mris: &[String]) -> Value {
    Value::Array(mris.iter().map(|m| json!({ "mri": m })).collect())
}

/// Parses the `getpresence` answer.
pub fn parse_presence(v: &Value) -> Vec<Presence> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let mri = e.get("mri").and_then(Value::as_str)?.to_string();
                    let p = e.get("presence");
                    let s = |k: &str| {
                        p.and_then(|p| p.get(k))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    };
                    let note = p
                        .and_then(|p| p.pointer("/note/message"))
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty())
                        .map(|m| StatusNote {
                            text: unescape(m.strip_suffix(PINNED_NOTE).unwrap_or(m)),
                            expiry: p
                                .and_then(|p| p.pointer("/note/expiry"))
                                .and_then(Value::as_str)
                                .filter(|x| !x.starts_with("9999"))
                                .map(str::to_owned),
                            show_when_messaged: m.ends_with(PINNED_NOTE),
                        })
                        .filter(|n| !n.text.is_empty());
                    let work_location = p.and_then(|p| p.get("workLocation")).and_then(|w| {
                        let loc = w.get("location").unwrap_or(w);
                        match loc {
                            Value::String(s) if !s.is_empty() => Some(s.clone()),
                            Value::Number(n) => Some(n.to_string()),
                            _ => None,
                        }
                    });
                    Some(Presence {
                        mri,
                        availability: s("availability"),
                        activity: s("activity"),
                        ooo: p
                            .and_then(|p| p.pointer("/calendarData/isOutOfOffice"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        note,
                        work_location,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Body for `forceavailability`. `expiry_iso` is the optional `desiredExpirationTime`.
pub fn force_availability_body(a: Availability, expiry_iso: Option<&str>) -> Value {
    let mut b = json!({ "availability": a.as_str() });
    if a == Availability::Offline {
        b["activity"] = "OffWork".into();
    }
    if let Some(e) = expiry_iso {
        b["desiredExpirationTime"] = e.into();
    }
    b
}

/// Body for `publishnote`. Empty `text` clears the note.
pub fn status_note_body(text: &str, expiry_iso: Option<&str>, show_when_messaged: bool) -> Value {
    let mut message = escape(text);
    if show_when_messaged && !text.is_empty() {
        message.push_str(PINNED_NOTE);
    }
    json!({ "message": message, "expiry": expiry_iso.unwrap_or(NEVER_EXPIRES) })
}

/// Body for `PUT me/endpoints/`: this device's own presence, which the service aggregates with
/// the user's other devices (and which lets a forced status show, see chat plan 6.4).
/// `activity` is e.g. `Available`, `InACall` or `InAConferenceCall`.
pub fn endpoint_body(endpoint_id: &str, a: Availability, activity: &str) -> Value {
    json!({
        "id": endpoint_id,
        "availability": a.as_str(),
        "activity": activity,
        "activityReporting": "Transport",
        "deviceType": "Desktop",
    })
}

/// Body for `reportmyactivity`.
pub fn activity_body(endpoint_id: &str, active: bool) -> Value {
    json!({ "endpointId": endpoint_id, "isActive": active })
}

impl Client {
    /// One presence request with the audience token and client headers. `path` is relative to
    /// the regional presence service's `/v1/`.
    pub async fn presence_json(
        &self,
        context: &'static str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let url = format!("{}/{path}", self.presence_service().await);
        let corr = uuid::Uuid::new_v4().to_string();
        let (status, text) = self
            .request_as(
                &Audience::Presence,
                method,
                &url,
                body,
                &[
                    ("x-ms-client-user-agent", "Teams-V2-Desktop"),
                    ("x-ms-correlation-id", &corr),
                    ("x-ms-client-version", CLIENT_VERSION),
                    ("x-ms-endpoint-id", self.epid()),
                ],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status(context, status, &text));
        }
        if text.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }

    /// Presence of any number of people, looked up in batches.
    pub async fn get_presence(&self, mris: &[String]) -> Result<Vec<Presence>> {
        let mut out = vec![];
        for chunk in mris.chunks(LOOKUP_CHUNK) {
            let v = self
                .presence_json(
                    "presence",
                    Method::POST,
                    "presence/getpresence/",
                    Some(&lookup_body(chunk)),
                )
                .await?;
            out.extend(parse_presence(&v));
        }
        Ok(out)
    }

    /// Forces our status (visible to everyone). `expiry_iso` ends it automatically.
    pub async fn set_presence(&self, a: Availability, expiry_iso: Option<&str>) -> Result<()> {
        let body = force_availability_body(a, expiry_iso);
        self.presence_json(
            "set presence",
            Method::PUT,
            "me/forceavailability/",
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    /// Clears a forced status. An empty body is the only spelling the service accepts.
    pub async fn reset_presence(&self) -> Result<()> {
        self.presence_json("reset presence", Method::PUT, "me/forceavailability/", None)
            .await
            .map(|_| ())
    }

    /// Publishes our status note (empty text clears it).
    pub async fn set_status_note(
        &self,
        text: &str,
        expiry_iso: Option<&str>,
        show_when_messaged: bool,
    ) -> Result<()> {
        let body = status_note_body(text, expiry_iso, show_when_messaged);
        self.presence_json("status note", Method::PUT, "me/publishnote", Some(&body))
            .await
            .map(|_| ())
    }

    /// Publishes this device's presence (registers the endpoint on first use).
    pub async fn publish_endpoint(&self, a: Availability, activity: &str) -> Result<()> {
        let body = endpoint_body(self.epid(), a, activity);
        self.presence_json(
            "publish endpoint",
            Method::PUT,
            "me/endpoints/",
            Some(&body),
        )
        .await
        .map(|_| ())
    }

    /// Removes this device's presence (on sign-out or quit), so it no longer counts.
    pub async fn remove_endpoint(&self) -> Result<()> {
        let path = format!("me/endpoints/{}", self.epid());
        self.presence_json("remove endpoint", Method::DELETE, &path, None)
            .await
            .map(|_| ())
    }

    /// Tells the service whether this endpoint is active (`false` lets us go Away when idle).
    /// A 404 means the presence endpoint expired and must be registered again.
    pub async fn report_activity(&self, active: bool) -> Result<()> {
        let body = activity_body(self.epid(), active);
        self.presence_json(
            "report activity",
            Method::POST,
            "me/reportmyactivity/",
            Some(&body),
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_round_trips() {
        for a in [
            Availability::Available,
            Availability::Busy,
            Availability::DoNotDisturb,
            Availability::BeRightBack,
            Availability::Away,
            Availability::Offline,
        ] {
            assert_eq!(Availability::from_wire(a.as_str()), Some(a));
        }
        assert_eq!(
            Availability::from_wire("BusyIdle"),
            Some(Availability::Busy)
        );
        assert_eq!(Availability::from_wire("PresenceUnknown"), None);
    }

    #[test]
    fn lookup_body_is_a_list_of_mris() {
        let b = lookup_body(&["8:orgid:a".into(), "8:orgid:b".into()]);
        assert_eq!(b, json!([{"mri": "8:orgid:a"}, {"mri": "8:orgid:b"}]));
    }

    #[test]
    fn parses_presence_with_note_and_ooo() {
        let v = json!([
            {"mri": "8:orgid:a", "status": 200, "presence": {
                "availability": "Busy", "activity": "InACall",
                "note": {"message": "Heads down &amp; focused<pinnednote></pinnednote>", "expiry": "2030-01-01T10:00:00.000Z"},
                "calendarData": {"isOutOfOffice": true},
                "workLocation": {"location": "remote", "locationSource": "user"}}},
            {"mri": "8:orgid:b", "presence": {"availability": "Offline", "activity": "OffWork",
                "note": {"message": "", "expiry": "9999-12-31T23:59:59.999Z"}}},
            {"mri": "8:orgid:c", "presence": {"availability": "Away",
                "note": {"message": "Lunch", "expiry": "9999-12-31T23:59:59.999Z"}}},
            {"mri": "8:orgid:d"},
            {"presence": {"availability": "Busy"}}
        ]);
        let p = parse_presence(&v);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].availability, "Busy");
        assert_eq!(p[0].activity, "InACall");
        assert!(p[0].ooo);
        let note = p[0].note.as_ref().unwrap();
        assert_eq!(note.text, "Heads down & focused");
        assert!(note.show_when_messaged);
        assert_eq!(note.expiry.as_deref(), Some("2030-01-01T10:00:00.000Z"));
        assert_eq!(p[0].work_location.as_deref(), Some("remote"));
        assert_eq!(p[1].note, None);
        assert!(!p[1].ooo);
        let lunch = p[2].note.as_ref().unwrap();
        assert!(!lunch.show_when_messaged);
        assert_eq!(lunch.expiry, None);
        assert_eq!(p[3].availability, "");
        assert!(parse_presence(&json!({})).is_empty());
    }

    #[test]
    fn force_availability_bodies() {
        assert_eq!(
            force_availability_body(Availability::Busy, None),
            json!({"availability": "Busy"})
        );
        let off = force_availability_body(Availability::Offline, Some("2030-01-01T00:00:00.000Z"));
        assert_eq!(off["availability"], "Offline");
        assert_eq!(off["activity"], "OffWork");
        assert_eq!(off["desiredExpirationTime"], "2030-01-01T00:00:00.000Z");
    }

    #[test]
    fn status_note_bodies() {
        let b = status_note_body("a < b", None, false);
        assert_eq!(b["message"], "a &lt; b");
        assert_eq!(b["expiry"], NEVER_EXPIRES);
        let shown = status_note_body("Out", Some("2030-01-01T00:00:00.000Z"), true);
        assert_eq!(shown["message"], "Out<pinnednote></pinnednote>");
        assert_eq!(shown["expiry"], "2030-01-01T00:00:00.000Z");
        // Clearing never carries the suffix.
        assert_eq!(status_note_body("", None, true)["message"], "");
    }

    #[test]
    fn endpoint_body_carries_this_devices_presence() {
        assert_eq!(
            endpoint_body("ep-1", Availability::Busy, "InACall"),
            json!({"id": "ep-1", "availability": "Busy", "activity": "InACall",
                   "activityReporting": "Transport", "deviceType": "Desktop"})
        );
    }

    #[test]
    fn activity_body_names_the_endpoint() {
        assert_eq!(
            activity_body("ep-1", false),
            json!({"endpointId": "ep-1", "isActive": false})
        );
    }
}
