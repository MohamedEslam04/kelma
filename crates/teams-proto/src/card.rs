//! Profile card: the person details Teams shows when you click a name or avatar.
//!
//! The base is `fetchShortProfile` (name, email). Job title, department and office come from the
//! Delve "Loki" person card, which is best effort: it needs a regional host, answers 400
//! `ClientTypeMissing` without the `X-ClientType` header, and may hold no work details at all.
//! Presence, status note and work location come from [`crate::presence`].

use crate::broker::Audience;
use crate::client::Client;
use crate::client_info::TEAMS_ORIGIN;
use crate::error::{Error, Result};
use reqwest::Method;
use serde_json::Value;

/// What the profile card shows about a person.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileCard {
    pub mri: String,
    pub display_name: String,
    pub email: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub company: Option<String>,
    pub office_location: Option<String>,
}

/// The work details Loki reports for a person.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LokiDetails {
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub company: Option<String>,
    pub office_location: Option<String>,
}

/// Parses a Loki `person` answer (`{person:{…}}` in v2, the bare object in v1). Each field takes
/// the first source that has it. Key names inside `workDetails[].value` are not verified against
/// a populated account.
pub fn parse_loki_person(v: &Value) -> LokiDetails {
    let person = v.get("person").unwrap_or(v);
    let mut out = LokiDetails::default();
    for src in ["workDetails", "enhancedWorkDetails"] {
        let entries: Vec<&Value> = match person.get(src) {
            Some(Value::Array(a)) => a.iter().map(|e| e.get("value").unwrap_or(e)).collect(),
            Some(o @ Value::Object(_)) => vec![o.get("value").unwrap_or(o)],
            _ => vec![],
        };
        for e in entries {
            let pick = |keys: &[&str]| {
                keys.iter().find_map(|k| {
                    e.get(*k)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
            };
            out.job_title = out.job_title.take().or(pick(&["jobTitle", "title"]));
            out.department = out.department.take().or(pick(&["department"]));
            out.company = out.company.take().or(pick(&["companyName", "company"]));
            out.office_location =
                out.office_location
                    .take()
                    .or(pick(&["officeLocation", "office", "location"]));
        }
    }
    out
}

/// Directory details Graph keeps for a person: phones, office, and their manager.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirectoryDetails {
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub company: Option<String>,
    pub office_location: Option<String>,
    pub work_phone: Option<String>,
    pub mobile_phone: Option<String>,
    /// The manager's display name.
    pub manager: Option<String>,
}

/// `GET /v1.0/users/{oid}` with the fields the card shows.
pub fn directory_user_url(oid: &str) -> String {
    crate::graph::graph_url(
        &["users", oid],
        "$select=jobTitle,department,companyName,officeLocation,businessPhones,mobilePhone",
    )
}

/// `GET /v1.0/users/{oid}/manager`.
pub fn directory_manager_url(oid: &str) -> String {
    crate::graph::graph_url(&["users", oid, "manager"], "$select=displayName")
}

/// Parses a Graph `user` (and, when given, its manager).
pub fn parse_directory_user(user: &Value, manager: Option<&Value>) -> DirectoryDetails {
    use crate::graph::str_of;
    DirectoryDetails {
        job_title: str_of(user, "jobTitle"),
        department: str_of(user, "department"),
        company: str_of(user, "companyName"),
        office_location: str_of(user, "officeLocation"),
        work_phone: user["businessPhones"]
            .as_array()
            .and_then(|a| a.iter().filter_map(Value::as_str).find(|s| !s.is_empty()))
            .map(str::to_owned),
        mobile_phone: str_of(user, "mobilePhone"),
        manager: manager.and_then(|m| str_of(m, "displayName")),
    }
}

/// Loki host prefixes to try for a chat-service URL, the user's own region first.
pub fn loki_prefixes(chat_service: &str) -> Vec<&'static str> {
    let host = url::Url::parse(chat_service)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let own = match host.split('.').next().unwrap_or_default() {
        "amer" | "us" | "na" => Some("nam"),
        "emea" | "eu" | "uk" | "de" | "fr" => Some("eur"),
        "apac" | "in" | "au" | "jp" => Some("apc"),
        _ => None,
    };
    let mut out: Vec<&'static str> = own.into_iter().collect();
    for p in ["nam", "eur", "apc"] {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

impl Client {
    /// A person's directory entry and their manager, read from Graph (read only; UNVERIFIED live,
    /// see docs/protocol.md). A person without a manager (or one Graph will not show) has none.
    pub async fn directory_details(&self, mri: &str) -> Result<DirectoryDetails> {
        let oid = crate::people::oid_of_mri(mri)
            .ok_or_else(|| Error::Protocol("not a directory person".into()))?;
        let (user_url, manager_url) = (directory_user_url(oid), directory_manager_url(oid));
        let (user, manager) = tokio::join!(
            self.json_as(
                "directory user",
                &Audience::Graph,
                Method::GET,
                &user_url,
                None
            ),
            self.json_as(
                "directory manager",
                &Audience::Graph,
                Method::GET,
                &manager_url,
                None
            ),
        );
        Ok(parse_directory_user(&user?, manager.ok().as_ref()))
    }

    /// Name, email and (when Delve has them) job title, department and office of one person.
    pub async fn profile_card(&self, mri: &str) -> Result<ProfileCard> {
        let p = self
            .profiles(&[mri.to_string()])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Protocol("no profile for that person".into()))?;
        let loki = match p.email.as_deref() {
            Some(email) => self.loki_details(email).await.unwrap_or_else(|e| {
                tracing::debug!(error = %e, "no Delve details");
                LokiDetails::default()
            }),
            None => LokiDetails::default(),
        };
        Ok(ProfileCard {
            mri: p.mri,
            display_name: p.display_name,
            email: p.email,
            job_title: p.job_title.or(loki.job_title),
            department: p.department.or(loki.department),
            company: loki.company,
            office_location: loki.office_location,
        })
    }

    /// Work details from the Delve person card for an email address.
    pub async fn loki_details(&self, email: &str) -> Result<LokiDetails> {
        let enc = url::form_urlencoded::byte_serialize(email.as_bytes()).collect::<String>();
        let mut last = Error::Protocol("no Delve host answered".into());
        for prefix in loki_prefixes(&self.chat_service().await) {
            let url = format!(
                "https://{prefix}.loki.delve.office.com/api/v2/person?smtp={enc}&personaType=User&locale=en-gb"
            );
            let (status, text) = self
                .request_as(
                    &Audience::Loki,
                    Method::GET,
                    &url,
                    None,
                    &[
                        ("X-ClientType", "MicrosoftTeams"),
                        ("X-ClientScenario", "TeamsProfileCard"),
                        ("Origin", TEAMS_ORIGIN),
                    ],
                )
                .await?;
            match status {
                200..=299 => return Ok(parse_loki_person(&serde_json::from_str(&text)?)),
                // Wrong region: the service answers 403 or 404 on the others.
                403 | 404 => last = Error::status("delve person", status, &text),
                _ => return Err(Error::status("delve person", status, &text)),
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_work_details_from_any_source() {
        let v = json!({"person": {
            "aadObjectId": "oid",
            "workDetails": [
                {"source": "Directory", "value": {"jobTitle": "Engineer", "department": ""}},
                {"source": "Mailbox", "value": {"department": "R&D", "officeLocation": "Building 4"}}
            ],
            "enhancedWorkDetails": {"companyName": "Example Corp"}
        }});
        let d = parse_loki_person(&v);
        assert_eq!(d.job_title.as_deref(), Some("Engineer"));
        assert_eq!(d.department.as_deref(), Some("R&D"));
        assert_eq!(d.office_location.as_deref(), Some("Building 4"));
        assert_eq!(d.company.as_deref(), Some("Example Corp"));
    }

    #[test]
    fn empty_details_and_v1_shape() {
        let empty = json!({"person": {"enhancedWorkDetails": {}, "workDetails": [{"source": "Directory", "value": {}}]}});
        assert_eq!(parse_loki_person(&empty), LokiDetails::default());
        let v1 = json!({"workDetails": [{"value": {"title": "Lead"}}]});
        assert_eq!(parse_loki_person(&v1).job_title.as_deref(), Some("Lead"));
        assert_eq!(parse_loki_person(&json!(null)), LokiDetails::default());
    }

    #[test]
    fn directory_entry_and_manager() {
        assert_eq!(
            directory_user_url("00000000-0000-0000-0000-000000000003"),
            "https://graph.microsoft.com/v1.0/users/00000000-0000-0000-0000-000000000003?$select=jobTitle,department,companyName,officeLocation,businessPhones,mobilePhone"
        );
        assert_eq!(
            directory_manager_url("abc"),
            "https://graph.microsoft.com/v1.0/users/abc/manager?$select=displayName"
        );
        let user = json!({"jobTitle": "Engineer", "department": "", "officeLocation": "Building 4",
            "businessPhones": ["", "+1 555 0100"], "mobilePhone": null});
        let d = parse_directory_user(&user, Some(&json!({"displayName": "Lead Person"})));
        assert_eq!(d.job_title.as_deref(), Some("Engineer"));
        assert_eq!(d.department, None);
        assert_eq!(d.office_location.as_deref(), Some("Building 4"));
        assert_eq!(d.work_phone.as_deref(), Some("+1 555 0100"));
        assert_eq!(d.mobile_phone, None);
        assert_eq!(d.manager.as_deref(), Some("Lead Person"));
        assert_eq!(
            parse_directory_user(&json!({}), None),
            DirectoryDetails::default()
        );
    }

    #[test]
    fn own_region_comes_first() {
        assert_eq!(
            loki_prefixes("https://emea.ng.msg.teams.microsoft.com"),
            vec!["eur", "nam", "apc"]
        );
        assert_eq!(
            loki_prefixes("https://amer.ng.msg.teams.microsoft.com"),
            vec!["nam", "eur", "apc"]
        );
        assert_eq!(loki_prefixes("garbage"), vec!["nam", "eur", "apc"]);
    }
}
