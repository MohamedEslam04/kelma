//! Install, uninstall and update requests ("Add for me / to a team / to a chat / to a meeting",
//! "Manage your apps" › Remove / Update). Built from the web client's own request builders
//! (`createAppEntitlement`, `deleteAppEntitlement`, `upgradeAppEntitlement`). The app's screens
//! send them after a confirm dialog; `ironteams-cli` only prints them as dry runs. They have not
//! been sent against a real account yet (the shapes come from the web client's code).

use reqwest::Method;
use serde_json::Value;

use crate::model::enc;

/// A request to the app service. The builders only build it; `api::send` sends it (IronTeams's
/// screens do, after the user confirms), `ironteams-cli` prints it as a dry run.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub body: Option<Value>,
}

impl Request {
    /// The request as one line (method and URL), for logs.
    pub fn describe(&self) -> String {
        format!("{} {}", self.method, self.url)
    }

    /// The request as text: method, URL and a summary of the body (its top-level keys or its
    /// length), never the body itself, which can hold tenant data.
    pub fn dry_run(&self) -> String {
        let body = match &self.body {
            None => "no body".to_owned(),
            Some(Value::Object(m)) => format!(
                "body: object with {} keys ({})",
                m.len(),
                m.keys().take(8).cloned().collect::<Vec<_>>().join(", ")
            ),
            Some(Value::Array(a)) => format!("body: array of {}", a.len()),
            Some(other) => format!("body: {other}"),
        };
        format!("DRY RUN {} {}\n  {body}", self.method, self.url)
    }
}

/// Where an app is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Personal scope ("Add for me").
    Me,
    /// A team, by its thread id (`19:…@thread.tacv2`).
    Team(String),
    /// A group chat, by its thread id.
    Chat(String),
    /// A meeting, by its chat thread id: installing to a meeting chat installs to the meeting.
    Meeting(String),
}

impl Target {
    /// The `{scope}` of the app service paths.
    fn scope(&self) -> String {
        match self {
            Target::Me => "users".to_owned(),
            Target::Team(id) => format!("teams/{}", enc(id)),
            Target::Chat(id) | Target::Meeting(id) => format!("chats/{}", enc(id)),
        }
    }
}

/// Installs an app: `POST beta/{scope}/apps/entitlements`. The web client posts the app's
/// definition as served by the app service (`AppDefinition::raw`).
pub fn install_request(base: &str, target: &Target, definition: &Value) -> Request {
    Request {
        method: Method::POST,
        url: format!("{base}/beta/{}/apps/entitlements", target.scope()),
        body: Some(definition.clone()),
    }
}

/// Removes an app: `DELETE beta/{scope}/apps/entitlements/{appId}`.
pub fn uninstall_request(base: &str, target: &Target, app_id: &str) -> Request {
    Request {
        method: Method::DELETE,
        url: format!(
            "{base}/beta/{}/apps/entitlements/{}",
            target.scope(),
            enc(app_id)
        ),
        body: None,
    }
}

/// Updates an app to `version`: `POST beta/{scope}/apps/Upgrade/{appId}?version=…`.
pub fn update_request(base: &str, target: &Target, app_id: &str, version: &str) -> Request {
    Request {
        method: Method::POST,
        url: format!(
            "{base}/beta/{}/apps/Upgrade/{}?version={}",
            target.scope(),
            enc(app_id),
            enc(version)
        ),
        body: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MT: &str = "https://teams.microsoft.com/api/mt/amer";

    #[test]
    fn install_posts_the_definition_to_the_scope() {
        let def = json!({"id": "app-1", "version": "1.0"});
        let me = install_request(MT, &Target::Me, &def);
        assert_eq!(me.method, Method::POST);
        assert_eq!(me.url, format!("{MT}/beta/users/apps/entitlements"));
        assert_eq!(me.body.as_ref(), Some(&def));
        let team = install_request(MT, &Target::Team("19:abc@thread.tacv2".into()), &def);
        assert_eq!(
            team.url,
            format!("{MT}/beta/teams/19%3Aabc%40thread.tacv2/apps/entitlements")
        );
        let meeting = install_request(MT, &Target::Meeting("19:meeting_x@thread.v2".into()), &def);
        assert_eq!(
            meeting.url,
            format!("{MT}/beta/chats/19%3Ameeting_x%40thread.v2/apps/entitlements")
        );
    }

    #[test]
    fn uninstall_and_update() {
        let r = uninstall_request(MT, &Target::Me, "com.microsoft.teamspace.tab.planner");
        assert_eq!(r.method, Method::DELETE);
        assert_eq!(
            r.url,
            format!("{MT}/beta/users/apps/entitlements/com.microsoft.teamspace.tab.planner")
        );
        assert!(r.body.is_none());
        let r = update_request(MT, &Target::Chat("19:c@thread.v2".into()), "a b", "2.0");
        assert_eq!(
            r.url,
            format!("{MT}/beta/chats/19%3Ac%40thread.v2/apps/Upgrade/a%20b?version=2.0")
        );
    }

    #[test]
    fn dry_run_summarises_the_body() {
        let r = install_request(MT, &Target::Me, &json!({"id": "x", "name": "secret"}));
        let s = r.dry_run();
        assert!(s.starts_with("DRY RUN POST https://"));
        assert!(s.contains("object with 2 keys (id, name)"));
        assert!(!s.contains("secret"));
        let d = uninstall_request(MT, &Target::Me, "x").dry_run();
        assert!(d.contains("DELETE") && d.contains("no body"));
        assert_eq!(
            uninstall_request(MT, &Target::Me, "x").describe(),
            format!("DELETE {MT}/beta/users/apps/entitlements/x")
        );
    }
}
