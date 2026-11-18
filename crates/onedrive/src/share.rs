//! Sharing: invite people, create links, manage who has access.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::time::parse_iso_ms;
use teams_proto::{Client, Error, Result};

use crate::api::{ItemRef, graph};

/// Who a link works for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkScope {
    /// Anyone with the link.
    Anonymous,
    /// People in the organization.
    Organization,
    /// Specific people (the link grants nothing until people are added).
    Users,
}

impl LinkScope {
    fn as_str(self) -> &'static str {
        match self {
            LinkScope::Anonymous => "anonymous",
            LinkScope::Organization => "organization",
            LinkScope::Users => "users",
        }
    }
}

/// What a link allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkType {
    View,
    Edit,
}

/// Access level for [`invite`] and [`update_permission`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Read,
    Write,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::Read => "read",
            Role::Write => "write",
        }
    }
}

/// A person to invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    Email(String),
    /// Entra object id.
    ObjectId(String),
}

/// Who or what has access to an item.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Permission {
    pub id: String,
    /// `read`, `write`, `owner`.
    pub roles: Vec<String>,
    /// Display names or emails of the people it was granted to.
    pub granted_to: Vec<String>,
    /// For a sharing link: `view`, `edit`, `blocksDownload`, ...
    pub link_type: Option<String>,
    /// For a sharing link: `anonymous`, `organization`, `users`.
    pub link_scope: Option<String>,
    pub link_url: Option<String>,
    pub expires_ms: Option<i64>,
    pub inherited: bool,
}

/// Body of `POST …/invite`.
pub fn invite_body(
    recipients: &[Recipient],
    role: Role,
    message: &str,
    send_invitation: bool,
) -> Value {
    let recipients: Vec<Value> = recipients
        .iter()
        .map(|r| match r {
            Recipient::Email(e) => json!({"email": e}),
            Recipient::ObjectId(o) => json!({"objectId": o}),
        })
        .collect();
    json!({
        "recipients": recipients,
        "message": message,
        "requireSignIn": true,
        "sendInvitation": send_invitation,
        "roles": [role.as_str()],
    })
}

/// Body of `POST …/createLink`. A view link that blocks download has the type `blocksDownload`;
/// an edit link cannot block download.
pub fn link_body(
    scope: LinkScope,
    kind: LinkType,
    expiry: Option<&str>,
    block_download: bool,
) -> Result<Value> {
    let ty = match (kind, block_download) {
        (LinkType::View, false) => "view",
        (LinkType::View, true) => "blocksDownload",
        (LinkType::Edit, false) => "edit",
        (LinkType::Edit, true) => {
            return Err(Error::Protocol(
                "a link that allows editing cannot block download".into(),
            ));
        }
    };
    let mut b = json!({"type": ty, "scope": scope.as_str()});
    if let Some(e) = expiry {
        b["expirationDateTime"] = json!(e);
    }
    Ok(b)
}

/// Parses one Graph `permission`.
pub fn parse_permission(v: &Value) -> Option<Permission> {
    let mut granted: Vec<String> = Vec::new();
    let mut add = |id: &Value| {
        for k in ["user", "siteUser", "group", "application"] {
            if let Some(n) = id[k]["displayName"].as_str().or(id[k]["email"].as_str()) {
                granted.push(n.to_owned());
                return;
            }
        }
    };
    add(&v["grantedToV2"]);
    for i in v["grantedToIdentitiesV2"].as_array().into_iter().flatten() {
        add(i);
    }
    let link = &v["link"];
    Some(Permission {
        id: v["id"].as_str()?.to_owned(),
        roles: v["roles"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        granted_to: granted,
        link_type: link["type"].as_str().map(str::to_owned),
        link_scope: link["scope"].as_str().map(str::to_owned),
        link_url: link["webUrl"].as_str().map(str::to_owned),
        expires_ms: v["expirationDateTime"].as_str().and_then(parse_iso_ms),
        inherited: v.get("inheritedFrom").is_some_and(|i| !i.is_null()),
    })
}

fn parse_permissions(v: &Value) -> Vec<Permission> {
    v["value"]
        .as_array()
        .map(|a| a.iter().filter_map(parse_permission).collect())
        .unwrap_or_default()
}

/// Shares `item` with people, optionally emailing them.
pub async fn invite(
    client: &Client,
    item: &ItemRef,
    recipients: &[Recipient],
    role: Role,
    message: &str,
    send_invitation: bool,
) -> Result<Vec<Permission>> {
    let body = invite_body(recipients, role, message, send_invitation);
    let v = graph(
        client,
        "invite",
        Method::POST,
        &format!("{}/invite", item.url()),
        Some(&body),
    )
    .await?;
    Ok(parse_permissions(&v))
}

/// Creates (or returns the existing) sharing link.
pub async fn create_link(
    client: &Client,
    item: &ItemRef,
    scope: LinkScope,
    kind: LinkType,
    expiry: Option<&str>,
    block_download: bool,
) -> Result<Permission> {
    let body = link_body(scope, kind, expiry, block_download)?;
    let v = graph(
        client,
        "create_link",
        Method::POST,
        &format!("{}/createLink", item.url()),
        Some(&body),
    )
    .await?;
    parse_permission(&v).ok_or_else(|| Error::Protocol("createLink returned no permission".into()))
}

/// Everyone and every link with access to `item`.
pub async fn permissions(client: &Client, item: &ItemRef) -> Result<Vec<Permission>> {
    let v = graph(
        client,
        "permissions",
        Method::GET,
        &format!("{}/permissions", item.url()),
        None,
    )
    .await?;
    Ok(parse_permissions(&v))
}

/// Changes the access level of one permission.
pub async fn update_permission(
    client: &Client,
    item: &ItemRef,
    permission_id: &str,
    role: Role,
) -> Result<()> {
    let body = json!({"roles": [role.as_str()]});
    graph(
        client,
        "update_permission",
        Method::PATCH,
        &format!("{}/permissions/{permission_id}", item.url()),
        Some(&body),
    )
    .await
    .map(|_| ())
}

/// Stops sharing: removes one permission or link.
pub async fn remove_permission(client: &Client, item: &ItemRef, permission_id: &str) -> Result<()> {
    graph(
        client,
        "remove_permission",
        Method::DELETE,
        &format!("{}/permissions/{permission_id}", item.url()),
        None,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_request_body() {
        let b = invite_body(
            &[
                Recipient::Email("ada@contoso.com".into()),
                Recipient::ObjectId("guid-1".into()),
            ],
            Role::Write,
            "Have a look",
            true,
        );
        assert_eq!(
            b,
            json!({
                "recipients": [{"email": "ada@contoso.com"}, {"objectId": "guid-1"}],
                "message": "Have a look",
                "requireSignIn": true,
                "sendInvitation": true,
                "roles": ["write"]
            })
        );
        assert_eq!(
            invite_body(&[], Role::Read, "", false)["roles"],
            json!(["read"])
        );
    }

    #[test]
    fn link_request_body() {
        assert_eq!(
            link_body(LinkScope::Organization, LinkType::View, None, false).unwrap(),
            json!({"type": "view", "scope": "organization"})
        );
        assert_eq!(
            link_body(
                LinkScope::Anonymous,
                LinkType::Edit,
                Some("2026-12-01T00:00:00Z"),
                false
            )
            .unwrap(),
            json!({"type": "edit", "scope": "anonymous", "expirationDateTime": "2026-12-01T00:00:00Z"})
        );
        assert_eq!(
            link_body(LinkScope::Users, LinkType::View, None, true).unwrap()["type"],
            "blocksDownload"
        );
        assert!(link_body(LinkScope::Users, LinkType::Edit, None, true).is_err());
    }

    #[test]
    fn parses_people_and_links() {
        let people = parse_permission(&json!({
            "id": "p1", "roles": ["write"],
            "grantedToV2": {"user": {"displayName": "Grace Hopper", "email": "grace@contoso.com"}},
            "inheritedFrom": {"id": "01X"}
        }))
        .unwrap();
        assert_eq!(people.granted_to, ["Grace Hopper"]);
        assert!(people.inherited);
        assert_eq!(people.roles, ["write"]);
        let link = parse_permission(&json!({
            "id": "p2", "roles": ["read"],
            "link": {"type": "view", "scope": "organization", "webUrl": "https://c.sharepoint.com/:w:/s/x"},
            "expirationDateTime": "2026-12-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(link.link_scope.as_deref(), Some("organization"));
        assert_eq!(
            link.link_url.as_deref(),
            Some("https://c.sharepoint.com/:w:/s/x")
        );
        assert!(link.expires_ms.is_some() && !link.inherited);
        let many = parse_permissions(&json!({"value": [{"id": "a"}, {"nope": 1}]}));
        assert_eq!(many.len(), 1);
        let ids = parse_permission(&json!({
            "id": "p3", "grantedToIdentitiesV2": [{"user": {"email": "a@x.com"}}, {"group": {"displayName": "Team"}}]
        }))
        .unwrap();
        assert_eq!(ids.granted_to, ["a@x.com", "Team"]);
    }
}
