//! Middle tier: user profiles and avatars.

use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::{Value, json};

/// A user's display profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub mri: String,
    pub display_name: String,
    pub email: Option<String>,
}

/// Parses a `fetchShortProfile` response.
pub fn parse_profiles(v: &Value) -> Vec<Profile> {
    v.get("value")
        .or_else(|| v.get("resolvedUsers"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let mri = p.get("mri").and_then(Value::as_str)?.to_string();
                    let display_name = p
                        .get("displayName")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .or_else(|| {
                            let g = p
                                .get("givenName")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let s = p.get("surname").and_then(Value::as_str).unwrap_or_default();
                            let full = format!("{g} {s}").trim().to_string();
                            (!full.is_empty()).then_some(full)
                        })
                        .unwrap_or_else(|| mri.clone());
                    let email = p
                        .get("email")
                        .or_else(|| p.get("userPrincipalName"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    Some(Profile {
                        mri,
                        display_name,
                        email,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Client {
    /// Fetches display names for up to ~100 MRIs (`8:orgid:…`) per call.
    pub async fn profiles(&self, mris: &[String]) -> Result<Vec<Profile>> {
        let mut out = vec![];
        let mt = self.middle_tier().await;
        for chunk in mris.chunks(100) {
            let url = format!(
                "{mt}/beta/users/fetchShortProfile?isMailAddress=false&enableGuest=true&includeIBBarredUsers=true&skypeTeamsInfo=true&includeBots=true"
            );
            let v = self
                .json("profiles", Method::POST, &url, Some(&json!(chunk)))
                .await?;
            out.extend(parse_profiles(&v));
        }
        Ok(out)
    }

    /// Downloads a user's avatar (JPEG/PNG bytes). `self_mri` is our own MRI.
    pub async fn avatar(&self, self_mri: &str, mri: &str) -> Result<Vec<u8>> {
        let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        let self_guid = self_mri
            .trim_start_matches("8:")
            .trim_start_matches("orgid:");
        let url = format!(
            "{}/beta/users/{}/profilepicturev2/{}?size=HR64x64",
            self.middle_tier().await,
            enc(self_guid),
            enc(mri)
        );
        self.bytes(&url, &[], 2 * 1024 * 1024).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profiles() {
        let p = parse_profiles(&json!({"value": [
            {"mri": "8:orgid:a", "displayName": "Sara Hassan", "email": "sara@x.com"},
            {"mri": "8:orgid:b", "displayName": "", "givenName": "Omar", "surname": "Khaled"},
            {"mri": "8:orgid:c"},
            {"displayName": "no mri"}
        ]}));
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].email.as_deref(), Some("sara@x.com"));
        assert_eq!(p[1].display_name, "Omar Khaled");
        assert_eq!(p[2].display_name, "8:orgid:c");
    }
}
