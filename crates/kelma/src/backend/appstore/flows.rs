//! Workflows (Power Automate): your flows and a flow's run history, with Run (a manual flow) and
//! Turn on or off. The write requests are UNVERIFIED against a real account (see
//! `teams_proto::workflows`), so a refusal is shown as it came.

use super::form::{self, Pending};
use super::view::{FlowsData, Row, day, duration, moment};
use crate::backend::Engine;
use std::sync::Arc;
use teams_proto::workflows::{
    Flow, FlowEnvironment, FlowRun, run_request, start_request, stop_request,
};

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

/// A flow with a button trigger can be run by hand.
fn is_manual(f: &Flow) -> bool {
    matches!(f.trigger.as_deref(), Some("Button" | "Manual" | "Request"))
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
            selected_id: selected.map(|f| f.id.clone()).unwrap_or_default(),
            status,
            loading: self.loading || self.runs_loading,
            can_run: selected.is_some_and(|f| f.is_on() && is_manual(f)),
            can_toggle: selected.is_some_and(|f| matches!(f.state.as_str(), "Started" | "Stopped")),
            selected_on: selected.is_some_and(Flow::is_on),
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
        let load = async {
            let env = self
                .src_flow_environments()
                .await?
                .into_iter()
                .max_by_key(|e| e.is_default)
                .ok_or_else(|| anyhow::anyhow!("no Power Automate environment"))?;
            let flows = self.src_flows(&env.name).await?;
            anyhow::Ok((env, flows))
        };
        let result: anyhow::Result<(FlowEnvironment, Vec<Flow>)> =
            super::tasks::limited("Power Automate", super::tasks::LOAD_LIMIT, load).await;
        match &result {
            Ok((_, flows)) => tracing::info!(flows = flows.len(), "flows loaded"),
            Err(e) => tracing::warn!(err = %e, "flows could not be loaded"),
        }
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

impl Engine {
    /// The selected flow and the environment it lives in.
    fn flow_target(&self, id: &str) -> Option<(Flow, String)> {
        let a = self.apps.lock().unwrap();
        let f = a.flows.flows.iter().find(|f| f.id == id)?.clone();
        Some((f, a.flows.env.as_ref()?.name.clone()))
    }

    pub(super) fn flows_begin_run(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let Some((f, _)) = self.flow_target(id) else {
            return Ok(());
        };
        let spec = form::confirm(
            format!("Run {}?", f.display_name),
            "The flow starts once now, as you.".into(),
            "Run",
            false,
        );
        self.form_open(Pending::FlowRun(id.to_owned()), spec, String::new());
        Ok(())
    }

    pub(super) fn flows_begin_toggle(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let Some((f, _)) = self.flow_target(id) else {
            return Ok(());
        };
        let (verb, body) = if f.is_on() {
            (
                "Turn off",
                "It stops running on its trigger until you turn it on again.",
            )
        } else {
            ("Turn on", "It starts running on its trigger.")
        };
        let spec = form::confirm(
            format!("{verb} {}?", f.display_name),
            body.into(),
            verb,
            f.is_on(),
        );
        self.form_open(Pending::FlowToggle(id.to_owned()), spec, String::new());
        Ok(())
    }

    pub(super) async fn flows_do_run(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let (f, env) = self
            .flow_target(id)
            .ok_or("This flow is not in the list.")?;
        match self.src_write("run flow", run_request(&env, id)).await {
            Ok(()) => {
                self.ui.banner(format!("Started {}.", f.display_name));
                let _ = self.flows_select(id).await;
            }
            Err(err) => {
                tracing::warn!(%err, "run flow failed");
                self.ui
                    .banner(format!("Could not run {}: {err}", f.display_name));
            }
        }
        Ok(())
    }

    pub(super) async fn flows_do_toggle(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let (f, env) = self
            .flow_target(id)
            .ok_or("This flow is not in the list.")?;
        let (w, done, failed) = if f.is_on() {
            (
                stop_request(&env, id),
                format!("Turned off {}.", f.display_name),
                format!("Could not turn off {}", f.display_name),
            )
        } else {
            (
                start_request(&env, id),
                format!("Turned on {}.", f.display_name),
                format!("Could not turn on {}", f.display_name),
            )
        };
        match self.src_write("flow state", w).await {
            Ok(()) => {
                self.ui.banner(done);
                // The service decides the state: read the list again.
                self.apps.lock().unwrap().flows.loaded = false;
                let _ = self.flows_refresh().await;
            }
            Err(err) => {
                tracing::warn!(%err, "{failed}");
                self.ui.banner(format!("{failed}: {err}"));
            }
        }
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
        assert_eq!(badges, ["On", "On", "Off", "On"]);
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
        // Only a flow with a button trigger that is on can be run by hand.
        assert!(!d.can_run && d.can_toggle && d.selected_on);
        let mut f = f;
        f.selected = Some("flow-4".into());
        assert!(f.data().can_run);
        f.selected = Some("flow-3".into());
        let d = f.data();
        assert!(
            !d.can_run && d.can_toggle && !d.selected_on,
            "off: turn on, no run"
        );
    }
}
