//! Workflows (Power Automate): environments, the user's flows and a flow's run history, plus the
//! requests that turn a flow on or off and run a manual one. The token audience is
//! `service.flow.microsoft.com`; the API host is `api.flow.microsoft.com`.
//!
//! The reads are verified live. The writes are NOT verified: `start` and `stop` follow the
//! Power Automate management API (`POST …/flows/{flow}/start` and `…/stop`); the manual run
//! (`POST …/flows/{flow}/triggers/manual/run`) is the request the maker portal's "Run" button is
//! known to send, and the trigger name `manual` is the default of "Manually trigger a flow".
//! None of them was sent against a real account.

use crate::broker::Audience;
use crate::client::Client;
use crate::error::Result;
use crate::graph::{Write, ms_of, str_of, values};
use reqwest::Method;
use serde_json::Value;

const API: &str = "https://api.flow.microsoft.com/providers/Microsoft.ProcessSimple";
const VERSION: &str = "api-version=2016-11-01";

/// A Power Automate environment (the default one holds the user's flows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowEnvironment {
    pub name: String,
    pub display_name: String,
    pub is_default: bool,
}

/// One of the user's flows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    pub id: String,
    pub display_name: String,
    /// `Started`, `Stopped` or `Suspended`.
    pub state: String,
    pub created_ms: Option<i64>,
    pub modified_ms: Option<i64>,
    /// The trigger kind (`Button`, `TeamsWebhook`, …) or its type (`Recurrence`, …).
    pub trigger: Option<String>,
    pub action_count: usize,
}

impl Flow {
    pub fn is_on(&self) -> bool {
        self.state == "Started"
    }
}

/// One run of a flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRun {
    pub id: String,
    /// `Succeeded`, `Failed`, `Running`, `Cancelled`, …
    pub status: String,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub trigger: Option<String>,
}

/// Environments from the list answer.
pub fn parse_environments(v: &Value) -> Vec<FlowEnvironment> {
    values(v)
        .iter()
        .filter_map(|e| {
            let p = &e["properties"];
            Some(FlowEnvironment {
                name: str_of(e, "name")?,
                display_name: str_of(p, "displayName").unwrap_or_default(),
                is_default: p["isDefault"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// Flows from the list answer.
pub fn parse_flows(v: &Value) -> Vec<Flow> {
    values(v)
        .iter()
        .filter_map(|f| {
            let p = &f["properties"];
            let summary = &p["definitionSummary"];
            let trigger = summary["triggers"]
                .get(0)
                .and_then(|t| str_of(t, "kind").or_else(|| str_of(t, "type")));
            Some(Flow {
                id: str_of(f, "name")?,
                display_name: str_of(p, "displayName").unwrap_or_default(),
                state: str_of(p, "state").unwrap_or_default(),
                created_ms: ms_of(p, "createdTime"),
                modified_ms: ms_of(p, "lastModifiedTime"),
                trigger,
                action_count: summary["actions"].as_array().map_or(0, Vec::len),
            })
        })
        .collect()
}

/// Runs from the run-history answer.
pub fn parse_runs(v: &Value) -> Vec<FlowRun> {
    values(v)
        .iter()
        .filter_map(|r| {
            let p = &r["properties"];
            Some(FlowRun {
                id: str_of(r, "name")?,
                status: str_of(p, "status").unwrap_or_default(),
                start_ms: ms_of(p, "startTime"),
                end_ms: ms_of(p, "endTime"),
                trigger: str_of(&p["trigger"], "name"),
            })
        })
        .collect()
}

fn url(path: &[&str], extra: &str) -> String {
    let mut u = url::Url::parse(API).expect("static url");
    u.path_segments_mut()
        .expect("https url has a path")
        .extend(path);
    u.set_query(Some(&format!("{VERSION}{extra}")));
    u.into()
}

fn flow_write(env: &str, flow: &str, tail: &[&str], body: Option<Value>) -> Write {
    let mut path = vec!["environments", env, "flows", flow];
    path.extend(tail);
    let mut w = Write::graph(Method::POST, url(&path, ""), body);
    w.audience = Audience::Flow;
    w
}

/// Turns a flow on: `POST …/flows/{flow}/start`. UNVERIFIED, never sent live.
pub fn start_request(env: &str, flow: &str) -> Write {
    flow_write(env, flow, &["start"], None)
}

/// Turns a flow off: `POST …/flows/{flow}/stop`. UNVERIFIED, never sent live.
pub fn stop_request(env: &str, flow: &str) -> Write {
    flow_write(env, flow, &["stop"], None)
}

/// Runs a manual (button) flow: `POST …/flows/{flow}/triggers/manual/run` with an empty
/// `inputs`. UNVERIFIED, never sent live.
pub fn run_request(env: &str, flow: &str) -> Write {
    flow_write(
        env,
        flow,
        &["triggers", "manual", "run"],
        Some(serde_json::json!({"inputs": {}})),
    )
}

impl Client {
    async fn flow_get(&self, context: &'static str, url: &str) -> Result<Value> {
        self.json_as(context, &Audience::Flow, Method::GET, url, None)
            .await
    }

    /// The Power Automate environments the user can see.
    pub async fn flow_environments(&self) -> Result<Vec<FlowEnvironment>> {
        let v = self
            .flow_get("flow environments", &url(&["environments"], ""))
            .await?;
        Ok(parse_environments(&v))
    }

    /// The flows the user owns or has access to in `environment`.
    pub async fn flows(&self, environment: &str) -> Result<Vec<Flow>> {
        let mut out = Vec::new();
        let mut next = Some(url(&["environments", environment, "flows"], ""));
        for _ in 0..10 {
            let Some(u) = next.take() else { break };
            let v = self.flow_get("flows", &u).await?;
            out.extend(parse_flows(&v));
            next = str_of(&v, "nextLink");
        }
        Ok(out)
    }

    /// The newest `top` runs of a flow, newest first.
    pub async fn flow_runs(
        &self,
        environment: &str,
        flow_id: &str,
        top: u32,
    ) -> Result<Vec<FlowRun>> {
        let u = url(
            &["environments", environment, "flows", flow_id, "runs"],
            &format!("&$top={top}"),
        );
        let v = self.flow_get("flow runs", &u).await?;
        Ok(parse_runs(&v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_environments() {
        let envs = parse_environments(&json!({"value":[
            {"name":"Default-1","properties":{"displayName":"Contoso (default)","isDefault":true}},
            {"properties":{}}]}));
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "Default-1");
        assert!(envs[0].is_default);
    }

    #[test]
    fn parses_flows() {
        let flows = parse_flows(&json!({"value":[
            {"name":"f1","properties":{"displayName":"Notify team","state":"Started",
              "createdTime":"2026-01-01T00:00:00Z","lastModifiedTime":"2026-01-02T00:00:00Z",
              "definitionSummary":{"triggers":[{"type":"Request","kind":"Button"}],
                                   "actions":[{"type":"Http"},{"type":"Compose"}]}}},
            {"name":"f2","properties":{"displayName":"Nightly","state":"Stopped",
              "definitionSummary":{"triggers":[{"type":"Recurrence"}],"actions":[]}}}]}));
        assert_eq!(flows[0].trigger.as_deref(), Some("Button"));
        assert_eq!(flows[0].action_count, 2);
        assert!(flows[0].is_on());
        assert_eq!(flows[0].modified_ms, Some(1_767_312_000_000));
        assert_eq!(flows[1].trigger.as_deref(), Some("Recurrence"));
        assert!(!flows[1].is_on());
        assert_eq!(flows[1].created_ms, None);
    }

    #[test]
    fn parses_runs() {
        let runs = parse_runs(&json!({"value":[
            {"name":"r1","properties":{"status":"Succeeded","startTime":"2026-01-02T00:00:00.1234567Z",
              "endTime":"2026-01-02T00:00:05Z","trigger":{"name":"manual"}}},
            {"name":"r2","properties":{"status":"Failed"}}],"nextLink":"x"}));
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].status, "Succeeded");
        assert_eq!(runs[0].trigger.as_deref(), Some("manual"));
        assert_eq!(runs[0].end_ms.unwrap() - runs[0].start_ms.unwrap(), 4_877);
        assert_eq!(runs[1].start_ms, None);
    }

    #[test]
    fn flow_writes_post_to_the_flow_with_the_flow_audience() {
        let on = start_request("Default-1", "f 1");
        assert_eq!(on.method, Method::POST);
        assert_eq!(on.audience, Audience::Flow);
        assert_eq!(
            on.url,
            "https://api.flow.microsoft.com/providers/Microsoft.ProcessSimple/environments/Default-1/flows/f%201/start?api-version=2016-11-01"
        );
        assert!(
            stop_request("e", "f")
                .url
                .contains("/flows/f/stop?api-version=")
        );
        let run = run_request("e", "f");
        assert!(
            run.url
                .contains("/flows/f/triggers/manual/run?api-version=")
        );
        assert_eq!(
            run.body,
            crate::graph::WriteBody::Json(serde_json::json!({"inputs": {}}))
        );
    }

    #[test]
    fn urls_carry_the_api_version() {
        assert_eq!(
            url(
                &["environments", "Default-1", "flows", "f 1", "runs"],
                "&$top=5"
            ),
            "https://api.flow.microsoft.com/providers/Microsoft.ProcessSimple/environments/Default-1/flows/f%201/runs?api-version=2016-11-01&$top=5"
        );
    }
}
