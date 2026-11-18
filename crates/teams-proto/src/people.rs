//! People search for "New chat" and the @mention picker.
//!
//! Primary route: Substrate people suggestions (`scenario=peoplepicker.newChat`). It answers 200
//! with no results unless `x-anchormailbox` names our own mailbox (`Oid:<oid>@<tenant id>`).
//! Fallback: the Teams middle tier `searchV2`. Sources: xteams, Maxim `substrate.ts`, rusty
//! `substrate_search` (see `docs/research/open-source.md` §3.2).

use crate::broker::Audience;
use crate::client::Client;
use crate::client_info::TEAMS_ORIGIN;
use crate::error::{Error, Result};
use crate::mt::{Profile, parse_profiles};
use base64::Engine;
use reqwest::Method;
use serde_json::{Value, json};

const SUGGESTIONS_URL: &str = "https://substrate.office.com/search/api/v1/suggestions";

/// A person found by a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// `8:orgid:<oid>` for people in the directory.
    pub mri: String,
    pub display_name: String,
    pub email: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
}

impl From<Profile> for Person {
    fn from(p: Profile) -> Person {
        Person {
            mri: p.mri,
            display_name: p.display_name,
            email: p.email,
            job_title: p.job_title,
            department: p.department,
        }
    }
}

/// The object id in `8:orgid:<oid>`.
pub fn oid_of_mri(mri: &str) -> Option<&str> {
    mri.strip_prefix("8:orgid:").filter(|s| !s.is_empty())
}

/// A claim from a JWT access token payload (no signature check: the token is our own).
pub fn jwt_claim(token: &str, claim: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get(claim).and_then(Value::as_str).map(str::to_owned)
}

/// Value of the `x-anchormailbox` header: `Oid:<oid>@<tenant id>`.
pub fn anchor_mailbox(self_mri: &str, tenant_id: &str) -> Option<String> {
    Some(format!("Oid:{}@{tenant_id}", oid_of_mri(self_mri)?))
}

/// Body for the Substrate people suggestions request.
pub fn people_search_body(query: &str, size: u32, cvid: &str, logical_id: &str) -> Value {
    json!({
        "EntityRequests": [{
            "Query": {"QueryString": query, "DisplayQueryString": query},
            "EntityType": "People",
            "Size": size,
            "Fields": [
                "Id", "MRI", "DisplayName", "EmailAddresses", "GivenName", "Surname", "JobTitle",
                "Department", "CompanyName", "UserPrincipalName", "ImAddress",
                "ExternalDirectoryObjectId"
            ],
            "Filter": {"And": [
                {"Or": [{"Term": {"PeopleType": "Person"}}, {"Term": {"PeopleType": "Other"}}]},
                {"Or": [
                    {"Term": {"PeopleSubtype": "OrganizationUser"}},
                    {"Term": {"PeopleSubtype": "MTOUser"}},
                    {"Term": {"PeopleSubtype": "Guest"}},
                    {"Term": {"PeopleSubtype": "PersonalContact"}}
                ]},
                {"Or": [{"Term": {"Flags": "NonHidden"}}]}
            ]},
            "Provenances": ["Mailbox", "Directory"],
            "From": 0
        }],
        "Cvid": cvid,
        "LogicalId": logical_id,
        "AppName": "Microsoft Teams",
        "Scenario": {"Name": "peoplepicker.newChat"}
    })
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Parses `Groups[].Suggestions[]` from a Substrate suggestions response. Entries without an
/// MRI or directory object id (for example personal contacts with no Teams account) are dropped.
pub fn parse_people_suggestions(v: &Value) -> Vec<Person> {
    let mut out: Vec<Person> = vec![];
    for s in v
        .get("Groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|g| {
            g.get("Suggestions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
    {
        let mri = text(s, "MRI")
            .or_else(|| text(s, "ExternalDirectoryObjectId").map(|oid| format!("8:orgid:{oid}")));
        let Some(mri) = mri else { continue };
        if out.iter().any(|p| p.mri == mri) {
            continue;
        }
        let email = s
            .get("EmailAddresses")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| text(s, "UserPrincipalName"));
        let given = text(s, "GivenName").unwrap_or_default();
        let surname = text(s, "Surname").unwrap_or_default();
        let display_name = text(s, "DisplayName")
            .or_else(|| {
                let full = format!("{given} {surname}").trim().to_string();
                (!full.is_empty()).then_some(full)
            })
            .unwrap_or_else(|| mri.clone());
        out.push(Person {
            mri,
            display_name,
            email,
            job_title: text(s, "JobTitle"),
            department: text(s, "Department"),
        });
    }
    out
}

impl Client {
    /// Our tenant id (the `tid` claim of our Teams token). `Session::tenant` may be only
    /// `organizations`, so this reads the real id from a token.
    pub async fn tenant_id(&self) -> Result<String> {
        let tok = self.token_for(&Audience::Spaces).await?;
        jwt_claim(&tok, "tid").ok_or_else(|| Error::Protocol("token has no tid claim".into()))
    }

    /// Searches the directory for people matching `query` (as typed in the "To:" field).
    /// Uses Substrate and falls back to the middle tier when Substrate fails.
    pub async fn search_people(&self, self_mri: &str, query: &str) -> Result<Vec<Person>> {
        match self.search_people_substrate(self_mri, query).await {
            Ok(p) => Ok(p),
            Err(e) => {
                tracing::warn!(error = %e, "substrate people search failed, trying the middle tier");
                self.search_people_mt(query).await
            }
        }
    }

    /// Substrate people suggestions for the "new chat" picker.
    pub async fn search_people_substrate(
        &self,
        self_mri: &str,
        query: &str,
    ) -> Result<Vec<Person>> {
        let tenant = self.tenant_id().await?;
        let anchor = anchor_mailbox(self_mri, &tenant)
            .ok_or_else(|| Error::Protocol(format!("{self_mri} is not an orgid MRI")))?;
        let cvid = uuid::Uuid::new_v4().to_string();
        let session = uuid::Uuid::new_v4().to_string();
        let body = people_search_body(query, 10, &cvid, &uuid::Uuid::new_v4().to_string());
        let url = format!("{SUGGESTIONS_URL}?scenario=peoplepicker.newChat");
        let (status, text) = self
            .request_as(
                &Audience::Substrate,
                Method::POST,
                &url,
                Some(&body),
                &[
                    ("Origin", TEAMS_ORIGIN),
                    ("Referer", &format!("{TEAMS_ORIGIN}/")),
                    ("x-anchormailbox", &anchor),
                    ("x-routingparameter-sessionkey", &session),
                    ("client-request-id", &cvid),
                    ("client-session-id", &session),
                    ("X-ClientType", "MicrosoftTeams"),
                    ("X-ClientScenario", "TeamsSearch"),
                ],
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("people search", status, &text));
        }
        Ok(parse_people_suggestions(&serde_json::from_str(&text)?))
    }

    /// Middle-tier people search (the fallback). The query is the whole request body.
    pub async fn search_people_mt(&self, query: &str) -> Result<Vec<Person>> {
        let url = format!(
            "{}/beta/users/searchV2?includeDLs=true&includeBots=true&enableGuest=true&source=newChat&skypeTeamsInfo=true",
            self.middle_tier().await
        );
        let v = self
            .json(
                "people search",
                Method::POST,
                &url,
                Some(&Value::from(query)),
            )
            .await?;
        Ok(parse_profiles(&v).into_iter().map(Person::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_from_mri() {
        assert_eq!(oid_of_mri("8:orgid:abc"), Some("abc"));
        assert_eq!(oid_of_mri("28:bot"), None);
        assert_eq!(oid_of_mri("8:orgid:"), None);
    }

    #[test]
    fn reads_claims_from_a_jwt() {
        let enc =
            |v: &Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
        let tok = format!(
            "{}.{}.sig",
            enc(&json!({"alg": "none"})),
            enc(&json!({"tid": "tenant-1", "n": 3}))
        );
        assert_eq!(jwt_claim(&tok, "tid").as_deref(), Some("tenant-1"));
        assert_eq!(jwt_claim(&tok, "n"), None);
        assert_eq!(jwt_claim(&tok, "missing"), None);
        assert_eq!(jwt_claim("not-a-jwt", "tid"), None);
    }

    #[test]
    fn anchor_header_uses_oid_and_tenant() {
        assert_eq!(
            anchor_mailbox("8:orgid:oid-1", "tid-9").as_deref(),
            Some("Oid:oid-1@tid-9")
        );
        assert_eq!(anchor_mailbox("8:live:x", "t"), None);
    }

    #[test]
    fn search_body_has_query_scenario_and_ids() {
        let b = people_search_body("sa", 10, "cv", "lg");
        let r = &b["EntityRequests"][0];
        assert_eq!(r["Query"]["QueryString"], "sa");
        assert_eq!(r["EntityType"], "People");
        assert_eq!(r["Size"], 10);
        assert_eq!(b["Scenario"]["Name"], "peoplepicker.newChat");
        assert_eq!(b["Cvid"], "cv");
        assert!(
            r["Fields"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "JobTitle")
        );
    }

    #[test]
    fn parses_suggestions() {
        let v = json!({"Groups": [{"Suggestions": [
            {"DisplayName": "Sara Hassan", "EmailAddresses": ["sara@example.test"],
             "ExternalDirectoryObjectId": "oid-a", "JobTitle": "Engineer", "Department": "R&D"},
            {"DisplayName": "Omar", "MRI": "8:orgid:oid-b", "UserPrincipalName": "omar@example.test"},
            {"GivenName": "No", "Surname": "Name", "ExternalDirectoryObjectId": "oid-c"},
            {"DisplayName": "Contact without an account"},
            {"DisplayName": "Duplicate of A", "ExternalDirectoryObjectId": "oid-a"}
        ]}, {"Suggestions": []}]});
        let p = parse_people_suggestions(&v);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].mri, "8:orgid:oid-a");
        assert_eq!(p[0].email.as_deref(), Some("sara@example.test"));
        assert_eq!(p[0].job_title.as_deref(), Some("Engineer"));
        assert_eq!(p[0].department.as_deref(), Some("R&D"));
        assert_eq!(p[1].mri, "8:orgid:oid-b");
        assert_eq!(p[1].email.as_deref(), Some("omar@example.test"));
        assert_eq!(p[2].display_name, "No Name");
        assert!(parse_people_suggestions(&json!({})).is_empty());
    }
}
