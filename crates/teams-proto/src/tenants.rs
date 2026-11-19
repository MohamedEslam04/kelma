//! Middle tier: the organizations (tenants) the signed-in account belongs to.

use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use serde_json::Value;

/// A tenant the account can work in: its home tenant or a guest tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    /// Primary domain, such as `contoso.com`.
    pub domain: Option<String>,
    /// The account's own organization (`tenantRelationship` is `Default`), as opposed to one it
    /// was invited to.
    pub is_home: bool,
    /// A guest invitation that was not accepted yet cannot be switched to.
    pub invitation_redeemed: bool,
    /// `member` or `guest` when the service says.
    pub user_type: Option<String>,
}

/// Parses the `users/tenants` answer (a bare array, or wrapped in `tenants` or `value`).
pub fn parse_tenants(v: &Value) -> Vec<Tenant> {
    let list = v
        .as_array()
        .or_else(|| v.get("tenants").and_then(Value::as_array))
        .or_else(|| v.get("value").and_then(Value::as_array));
    let text = |t: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| t.get(*k).and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    list.map(|a| {
        a.iter()
            .filter_map(|t| {
                let id = text(t, &["tenantId", "id"])?;
                Some(Tenant {
                    name: text(t, &["tenantName", "displayName", "name"])
                        .unwrap_or_else(|| id.clone()),
                    domain: text(t, &["tenantPrimaryDomainName"]),
                    is_home: text(t, &["tenantRelationship"])
                        .is_some_and(|r| r.eq_ignore_ascii_case("default")),
                    invitation_redeemed: t
                        .get("isInvitationRedeemed")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    user_type: text(t, &["userType"]),
                    id,
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

impl Client {
    /// Lists the home and guest tenants of the signed-in account.
    pub async fn tenants(&self) -> Result<Vec<Tenant>> {
        let mt = self.middle_tier().await;
        let v = self
            .json(
                "tenants",
                Method::GET,
                &format!("{mt}/beta/users/tenants"),
                None,
            )
            .await?;
        Ok(parse_tenants(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The answer's shape as seen live (ids and names invented).
    fn sample() -> Value {
        json!([
            {"countryLetterCode":"US","isInvitationRedeemed":true,"tenantId":"11111111-1111-1111-1111-111111111111",
             "tenantName":"Contoso","tenantPrimaryDomainName":"contoso.com","tenantRelationship":"Default",
             "tenantType":"organization","userId":"u1","userTenantState":"Default","userType":"member"},
            {"isInvitationRedeemed":false,"tenantId":"22222222-2222-2222-2222-222222222222",
             "tenantName":"Fabrikam","tenantPrimaryDomainName":"fabrikam.com","tenantRelationship":"Guest",
             "tenantType":"organization","userId":"u2","userTenantState":"Other","userType":"member"}
        ])
    }

    #[test]
    fn parses_home_and_guest_tenants() {
        let t = parse_tenants(&sample());
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(t[0].name, "Contoso");
        assert!(t[0].is_home && t[0].invitation_redeemed);
        assert_eq!(t[0].domain.as_deref(), Some("contoso.com"));
        assert!(
            !t[1].is_home,
            "a guest tenant is not home even when userType says member"
        );
        assert!(!t[1].invitation_redeemed);
        assert_eq!(t[1].name, "Fabrikam");
    }

    #[test]
    fn accepts_wrapped_lists_and_skips_entries_without_an_id() {
        let wrapped = json!({"tenants": [sample()[0].clone(), {"tenantName": "no id"}]});
        assert_eq!(parse_tenants(&wrapped).len(), 1);
        assert_eq!(parse_tenants(&json!({"value": []})), vec![]);
        assert_eq!(parse_tenants(&Value::Null), vec![]);
    }

    #[test]
    fn name_falls_back_to_the_id() {
        let t = parse_tenants(&json!([{"tenantId": "abc"}]));
        assert_eq!(t[0].name, "abc");
        assert!(
            t[0].invitation_redeemed,
            "assumed redeemed when the field is absent"
        );
    }
}
