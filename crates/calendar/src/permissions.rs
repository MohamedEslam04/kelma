//! Sharing my calendar (C3.3): who can see or edit it.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::{Client, Error, Result};

use crate::graph::{GraphRequest, email_address, enc, graph};
use crate::new_event::Person;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRole {
    None,
    /// Sees when I am busy.
    FreeBusy,
    /// Sees busy times, titles and places.
    LimitedRead,
    /// Sees all details.
    Read,
    /// Can edit.
    Write,
    /// Delegate: can act for me, private items hidden.
    Delegate,
    /// Delegate: can act for me and see private items.
    DelegateWithPrivate,
    Custom,
}

impl PermissionRole {
    pub fn graph(self) -> &'static str {
        match self {
            PermissionRole::None => "none",
            PermissionRole::FreeBusy => "freeBusyRead",
            PermissionRole::LimitedRead => "limitedRead",
            PermissionRole::Read => "read",
            PermissionRole::Write => "write",
            PermissionRole::Delegate => "delegateWithoutPrivateEventAccess",
            PermissionRole::DelegateWithPrivate => "delegateWithPrivateEventAccess",
            PermissionRole::Custom => "custom",
        }
    }

    pub fn from_graph(s: &str) -> PermissionRole {
        [
            PermissionRole::None,
            PermissionRole::FreeBusy,
            PermissionRole::LimitedRead,
            PermissionRole::Read,
            PermissionRole::Write,
            PermissionRole::Delegate,
            PermissionRole::DelegateWithPrivate,
        ]
        .into_iter()
        .find(|r| r.graph() == s)
        .unwrap_or(PermissionRole::Custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permission {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: PermissionRole,
    /// False for the owner's own entry.
    pub is_removable: bool,
    /// The roles this entry may be given; empty when Graph does not say.
    pub allowed_roles: Vec<PermissionRole>,
}

pub fn parse_permissions(v: &Value) -> Vec<Permission> {
    v.get("value")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|p| {
                    let addr = p.get("emailAddress").unwrap_or(&Value::Null);
                    let s = |v: &Value, k: &str| {
                        v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
                    };
                    Some(Permission {
                        id: Some(s(p, "id")).filter(|i| !i.is_empty())?,
                        name: s(addr, "name"),
                        email: s(addr, "address"),
                        role: PermissionRole::from_graph(&s(p, "role")),
                        is_removable: p
                            .get("isRemovable")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        allowed_roles: p
                            .get("allowedRoles")
                            .and_then(Value::as_array)
                            .map(|r| {
                                r.iter()
                                    .filter_map(Value::as_str)
                                    .map(PermissionRole::from_graph)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn add_permission_body(person: &Person, role: PermissionRole, inside_org: bool) -> Value {
    json!({
        "emailAddress": email_address(&person.name, &person.email),
        "isInsideOrganization": inside_org,
        "isRemovable": true,
        "role": role.graph(),
    })
}

pub fn update_permission_body(role: PermissionRole) -> Value {
    json!({"role": role.graph()})
}

fn path(calendar_id: &str, permission_id: Option<&str>) -> String {
    let base = format!("/me/calendars/{}/calendarPermissions", enc(calendar_id));
    match permission_id {
        Some(p) => format!("{base}/{}", enc(p)),
        None => base,
    }
}

/// Sharing the calendar with someone, built but not sent (see [`add_permission`]).
pub fn add_permission_request(
    calendar_id: &str,
    person: &Person,
    role: PermissionRole,
    inside_org: bool,
) -> GraphRequest {
    GraphRequest {
        method: Method::POST,
        path: path(calendar_id, None),
        body: Some(add_permission_body(person, role, inside_org)),
    }
}

/// Changing someone's role, built but not sent (see [`update_permission`]).
pub fn update_permission_request(
    calendar_id: &str,
    permission_id: &str,
    role: PermissionRole,
) -> GraphRequest {
    GraphRequest {
        method: Method::PATCH,
        path: path(calendar_id, Some(permission_id)),
        body: Some(update_permission_body(role)),
    }
}

/// Stopping sharing with someone, built but not sent (see [`remove_permission`]).
pub fn remove_permission_request(calendar_id: &str, permission_id: &str) -> GraphRequest {
    GraphRequest {
        method: Method::DELETE,
        path: path(calendar_id, Some(permission_id)),
        body: None,
    }
}

pub async fn calendar_permissions(client: &Client, calendar_id: &str) -> Result<Vec<Permission>> {
    let v = graph(
        client,
        "calendar permissions",
        Method::GET,
        &path(calendar_id, None),
        None,
    )
    .await?;
    Ok(parse_permissions(&v))
}

/// Shares the calendar with someone. Graph tells them by email.
pub async fn add_permission(
    client: &Client,
    calendar_id: &str,
    person: &Person,
    role: PermissionRole,
    inside_org: bool,
) -> Result<Permission> {
    let v = graph(
        client,
        "add permission",
        Method::POST,
        &path(calendar_id, None),
        Some(&add_permission_body(person, role, inside_org)),
    )
    .await?;
    parse_permissions(&json!({"value": [v]}))
        .into_iter()
        .next()
        .ok_or_else(|| Error::Protocol("add permission: no id in the response".into()))
}

pub async fn update_permission(
    client: &Client,
    calendar_id: &str,
    permission_id: &str,
    role: PermissionRole,
) -> Result<()> {
    graph(
        client,
        "update permission",
        Method::PATCH,
        &path(calendar_id, Some(permission_id)),
        Some(&update_permission_body(role)),
    )
    .await?;
    Ok(())
}

pub async fn remove_permission(
    client: &Client,
    calendar_id: &str,
    permission_id: &str,
) -> Result<()> {
    graph(
        client,
        "remove permission",
        Method::DELETE,
        &path(calendar_id, Some(permission_id)),
        None,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_round_trip() {
        for r in [
            PermissionRole::None,
            PermissionRole::FreeBusy,
            PermissionRole::LimitedRead,
            PermissionRole::Read,
            PermissionRole::Write,
            PermissionRole::Delegate,
            PermissionRole::DelegateWithPrivate,
        ] {
            assert_eq!(PermissionRole::from_graph(r.graph()), r);
        }
        assert_eq!(
            PermissionRole::from_graph("something new"),
            PermissionRole::Custom
        );
    }

    #[test]
    fn parses_permissions() {
        let p = parse_permissions(&json!({"value": [
            {"id": "p1", "emailAddress": {"name": "My Organization", "address": ""},
             "role": "freeBusyRead", "isRemovable": false},
            {"id": "p2", "emailAddress": {"name": "Sara", "address": "sara@contoso.example"},
             "role": "write", "isRemovable": true,
             "allowedRoles": ["freeBusyRead", "limitedRead", "read", "write", "delegateWithPrivateEventAccess"]},
            {"emailAddress": {}}]}));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].role, PermissionRole::FreeBusy);
        assert!(!p[0].is_removable);
        assert_eq!(p[1].email, "sara@contoso.example");
        assert_eq!(p[1].role, PermissionRole::Write);
        assert!(p[0].allowed_roles.is_empty());
        assert_eq!(p[1].allowed_roles.len(), 5);
        assert_eq!(p[1].allowed_roles[4], PermissionRole::DelegateWithPrivate);
    }

    #[test]
    fn dry_run_requests() {
        let add = add_permission_request(
            "c1",
            &Person::new("", "sara@contoso.example"),
            PermissionRole::FreeBusy,
            true,
        );
        assert_eq!(add.method, Method::POST);
        assert_eq!(add.path, "/me/calendars/c1/calendarPermissions");
        assert_eq!(add.body.as_ref().unwrap()["role"], "freeBusyRead");
        let up = update_permission_request("c1", "p=2", PermissionRole::Read);
        assert_eq!(up.method, Method::PATCH);
        assert_eq!(up.path, "/me/calendars/c1/calendarPermissions/p%3D2");
        assert_eq!(up.body, Some(json!({"role": "read"})));
        let rm = remove_permission_request("c1", "p2");
        assert_eq!(rm.method, Method::DELETE);
        assert_eq!(rm.body, None);
    }

    #[test]
    fn request_bodies_and_paths() {
        assert_eq!(
            add_permission_body(
                &Person::new("Sara", "sara@contoso.example"),
                PermissionRole::Read,
                true
            ),
            json!({"emailAddress": {"address": "sara@contoso.example", "name": "Sara"},
                   "isInsideOrganization": true, "isRemovable": true, "role": "read"})
        );
        assert_eq!(
            update_permission_body(PermissionRole::Write),
            json!({"role": "write"})
        );
        assert_eq!(
            path("c=1", Some("p/2")),
            "/me/calendars/c%3D1/calendarPermissions/p%2F2"
        );
    }
}
