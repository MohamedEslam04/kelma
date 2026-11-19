//! Workflows (Power Automate), read only: your flows and a flow's run history. kelma never turns
//! a flow on or off and never runs one.

use super::view::{FlowsData, Row, day, duration, moment};
use crate::backend::Engine;
use std::sync::Arc;
use teams_proto::workflows::{Flow, FlowEnvironment, FlowRun};

#[derive(Default)]
pub struct Flows {
    loaded: bool,
    env: Option<FlowEnvironment>,
    flows: Vec<Flow>,
    selected: Option<String>,
    runs: Vec<FlowRun>,
    status: String,
    loading: bool,
    runs_loading: bool,
}

/// `Button` as `Manual button`, `Recurrence` as `Scheduled`, and so on.
fn trigger_text(t: Option<&str>) -> String {
    match t.unwrap_or_default() {
        "" => "Unknown trigger".into(),
        "Button" | "Manual" | "Request" => "Manual button".into(),
        "Recurrence" => "Scheduled".into(),
        "TeamsWebhook" => "Teams".into(),
        other => other.to_owned(),
    }
}

fn run_tone(status: &str) -> &'static str {
    match status {
        "Succeeded" => "ok",
        "Failed" | "TimedOut" => "bad",
        "Running" | "Waiting" => "warn",
        _ => "",
    }
}

impl Flows {
    pub fn data(&self) -> FlowsData {
        let flows = self
            .flows
            .iter()
            .map(|f| Row {
                id: f.id.clone(),
                title: f.display_name.clone(),
                subtitle: format!(
                    "{} · {} action{}",
                    trigger_text(f.trigger.as_deref()),
                    f.action_count,
                    if f.action_count == 1 { "" } else { "s" }
                ),
                meta: f
                    .modified_ms
                    .map(|m| format!("Modified {}", day(m)))
                    .unwrap_or_default(),
                badge: if f.is_on() { "On".into() } else { "Off".into() },
                tone: if f.is_on() { "ok" } else { "" },
                selected: self.selected.as_deref() == Some(f.id.as_str()),
                ..Default::default()
            })
            .collect();
        let runs = self
            .runs
            .iter()
            .map(|r| Row {
                id: r.id.clone(),
                title: r
                    .start_ms
                    .map(moment)
                    .unwrap_or_else(|| "Unknown time".into()),
                subtitle: match (r.start_ms, r.end_ms) {
                    (Some(s), Some(e)) => duration(s, e),
                    _ => String::new(),
                },
                meta: r.trigger.clone().unwrap_or_default(),
                badge: r.status.clone(),
                tone: run_tone(&r.status),
                ..Default::default()
            })
            .collect();
        let selected = self
            .flows
            .iter()
            .find(|f| self.selected.as_deref() == Some(f.id.as_str()));
        let status = if !self.status.is_empty() {
            self.status.clone()
        } else if self.loaded && !self.loading && self.flows.is_empty() {
            "You have no flows.".into()
        } else if selected.is_some() && !self.runs_loading && self.runs.is_empty() {
            "This flow has not run yet.".into()
        } else {
            String::new()
        };
        FlowsData {
            flows,
            runs,
            environment: self
                .env
                .as_ref()
                .map(|e| e.display_name.clone())
                .unwrap_or_default(),
            selected_name: selected.map(|f| f.display_name.clone()).unwrap_or_default(),
            status,
            loading: self.loading || self.runs_loading,
        }
    }
}

impl Engine {
    fn flows_push(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().flows.data();
        self.ui.apps_flows(data);
    }

    fn flows_edit(self: &Arc<Self>, f: impl FnOnce(&mut Flows)) {
        f(&mut self.apps.lock().unwrap().flows);
        self.flows_push();
    }

    pub(super) async fn flows_enter(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("workflows".into());
        let busy = {
            let f = &self.apps.lock().unwrap().flows;
            f.loaded || f.loading
        };
        if busy {
            self.flows_push();
            return Ok(());
        }
        self.flows_refresh().await
    }

    pub(super) async fn flows_refresh(self: &Arc<Self>) -> anyhow::Result<()> {
        self.flows_edit(|f| {
            f.loading = true;
            f.status.clear();
        });
        let result: anyhow::Result<(FlowEnvironment, Vec<Flow>)> = async {
            let env = self
                .src_flow_environments()
                .await?
                .into_iter()
                .max_by_key(|e| e.is_default)
                .ok_or_else(|| anyhow::anyhow!("no Power Automate environment"))?;
            let flows = self.src_flows(&env.name).await?;
            Ok((env, flows))
        }
        .await;
        let first = {
            let mut a = self.apps.lock().unwrap();
            let f = &mut a.flows;
            f.loaded = true;
            f.loading = false;
            match result {
                Ok((env, flows)) => {
                    f.env = Some(env);
                    f.flows = flows;
                    f.selected = f
                        .selected
                        .clone()
                        .filter(|s| f.flows.iter().any(|x| x.id == *s))
                        .or_else(|| f.flows.first().map(|x| x.id.clone()));
                }
                Err(e) => f.status = format!("Loading your flows failed: {e}"),
            }
            f.selected.clone()
        };
        self.flows_push();
        if let Some(id) = first {
            self.flows_select(&id).await?;
        }
        Ok(())
    }

    /// Shows a flow's run history.
    pub(super) async fn flows_select(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let env = {
            let mut a = self.apps.lock().unwrap();
            a.flows.selected = Some(id.to_owned());
            a.flows.runs.clear();
            a.flows.runs_loading = true;
            a.flows.env.as_ref().map(|e| e.name.clone())
        };
        self.flows_push();
        let Some(env) = env else { return Ok(()) };
        let runs = self.src_runs(&env, id).await;
        self.flows_edit(|f| {
            // A newer selection may have been made while this one loaded.
            if f.selected.as_deref() != Some(id) {
                return;
            }
            f.runs_loading = false;
            match runs {
                Ok(r) => f.runs = r,
                Err(e) => f.status = format!("Loading the run history failed: {e}"),
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teams_proto::workflows::{parse_flows, parse_runs};

    #[test]
    fn flows_show_state_and_runs_show_status_tone() {
        let now = teams_proto::time::now_ms();
        let f = Flows {
            loaded: true,
            flows: parse_flows(&super::super::fixtures::flows(now)),
            runs: parse_runs(&super::super::fixtures::flow_runs("flow-1", now)),
            selected: Some("flow-1".into()),
            ..Default::default()
        };
        let d = f.data();
        let badges: Vec<&str> = d.flows.iter().map(|r| r.badge.as_str()).collect();
        assert_eq!(badges, ["On", "On", "Off"]);
        assert!(d.flows[0].selected && !d.flows[1].selected);
        assert_eq!(d.flows[2].subtitle, "Manual button · 2 actions");
        assert_eq!(d.flows[1].subtitle, "Scheduled · 1 action");
        let tones: Vec<(&str, &str)> = d.runs.iter().map(|r| (r.badge.as_str(), r.tone)).collect();
        assert_eq!(
            tones,
            [("Succeeded", "ok"), ("Failed", "bad"), ("Succeeded", "ok")]
        );
        assert_eq!(d.runs[0].subtitle, "4 s");
        assert!(d.selected_name.starts_with("Post a message"));
    }
}
