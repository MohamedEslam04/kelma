//! Where the app screens read from and write to: the live services or, in demo mode and in the
//! tests, the synthetic answers of `fixtures` (parsed by the same functions) and a log of the
//! writes that would have been sent. Offline, nothing leaves the process.

use super::fixtures as fx;
use crate::backend::Engine;
use app_store::{
    AppDefinition, AppPolicy, CatalogApp, InstalledApps, SearchHit, StoreHome, YourApp,
};
use std::sync::atomic::Ordering;
use teams_proto::graph::Write;
use teams_proto::lists::{Column, ListItem, Site, SpList};
use teams_proto::onenote::{Notebook, Page, Section};
use teams_proto::planner::{Bucket, Plan, PlannerTask, TodoList, TodoTask};
use teams_proto::recaps::{RecapMeeting, RecapResource, RecordingFile};
use teams_proto::time::now_ms;
use teams_proto::workflows::{Flow, FlowEnvironment, FlowRun};

type Res<T> = anyhow::Result<T>;

impl Engine {
    pub(super) fn apps_offline(&self) -> bool {
        self.offline.load(Ordering::Relaxed)
    }

    /// The app service base URL, for the requests that are only shown.
    pub(super) async fn apps_base(&self) -> String {
        if self.apps_offline() {
            "https://teams.example.invalid/api/mt/demo".into()
        } else {
            app_store::api::base(&self.client).await
        }
    }

    // -- writes --

    /// Sends a Graph or Power Automate change. Offline it is only logged.
    pub(super) async fn src_write(&self, context: &'static str, w: Write) -> Res<()> {
        tracing::info!(write = %w.describe(), "app write");
        if self.apps_offline() {
            self.apps.lock().unwrap().sent.push(w.describe());
            return Ok(());
        }
        self.client.send_write(context, &w).await?;
        Ok(())
    }

    /// Sends an app service change (install, remove, update, pin). Offline it is only logged.
    pub(super) async fn src_send(&self, context: &'static str, req: app_store::Request) -> Res<()> {
        tracing::info!(write = %req.describe(), "app write");
        if self.apps_offline() {
            self.apps.lock().unwrap().sent.push(req.describe());
            return Ok(());
        }
        app_store::api::send(&self.client, context, &req).await?;
        Ok(())
    }

    pub(super) async fn src_installed(&self) -> Res<InstalledApps> {
        if self.apps_offline() {
            return Ok(app_store::parse_installed(&fx::installed()));
        }
        Ok(app_store::installed_apps(&self.client).await?)
    }

    pub(super) async fn src_policy(&self) -> Res<AppPolicy> {
        if self.apps_offline() {
            return Ok(app_store::parse_app_policy(&fx::app_policy()));
        }
        Ok(app_store::app_policy(&self.client).await?)
    }

    pub(super) async fn src_home(&self) -> Res<StoreHome> {
        if self.apps_offline() {
            return Ok(app_store::parse_store_home(&fx::store_home()));
        }
        Ok(app_store::store_home(&self.client).await?)
    }

    pub(super) async fn src_search(&self, query: &str) -> Res<Vec<SearchHit>> {
        if self.apps_offline() {
            return Ok(app_store::parse_search(&fx::search(query)));
        }
        Ok(app_store::search(&self.client, query).await?)
    }

    pub(super) async fn src_defs(&self, ids: &[&str]) -> Res<Vec<AppDefinition>> {
        if self.apps_offline() {
            return Ok(app_store::parse_definitions(&fx::definitions(ids)));
        }
        Ok(app_store::definitions(&self.client, ids).await?)
    }

    pub(super) async fn src_mine(&self) -> Res<Vec<YourApp>> {
        if self.apps_offline() {
            return Ok(app_store::parse_your_apps(&fx::your_apps()));
        }
        Ok(app_store::your_apps(&self.client).await?)
    }

    pub(super) async fn src_org(&self) -> Res<Vec<CatalogApp>> {
        if self.apps_offline() {
            return Ok(app_store::catalog_apps(&fx::org_apps()));
        }
        Ok(app_store::org_apps(&self.client).await?)
    }

    // -- Planner and To Do --

    pub(super) async fn src_todo_lists(&self) -> Res<Vec<TodoList>> {
        if self.apps_offline() {
            return Ok(teams_proto::planner::parse_todo_lists(&fx::todo_lists()));
        }
        Ok(self.client.todo_lists().await?)
    }

    pub(super) async fn src_todo_tasks(&self, list: &str) -> Res<Vec<TodoTask>> {
        if self.apps_offline() {
            let v = fx::todo_tasks(list, now_ms());
            return Ok(teams_proto::planner::parse_todo_tasks(&v));
        }
        Ok(self.client.todo_tasks(list).await?)
    }

    pub(super) async fn src_plans(&self) -> Res<Vec<Plan>> {
        if self.apps_offline() {
            return Ok(teams_proto::planner::parse_plans(&fx::planner_plans()));
        }
        // Plans the user is on, plus those of the teams joined (a plan lives in its team's group).
        let mut plans = self.client.my_planner_plans().await?;
        for g in self.client.joined_group_ids().await.unwrap_or_default() {
            for p in self
                .client
                .group_planner_plans(&g)
                .await
                .unwrap_or_default()
            {
                if !plans.iter().any(|o| o.id == p.id) {
                    plans.push(p);
                }
            }
        }
        Ok(plans)
    }

    /// The signed-in user's tasks across plans (`plan` is `None`), or one plan's tasks.
    pub(super) async fn src_planner_tasks(&self, plan: Option<&str>) -> Res<Vec<PlannerTask>> {
        if self.apps_offline() {
            let v = fx::planner_tasks(plan, now_ms());
            return Ok(teams_proto::planner::parse_planner_tasks(&v));
        }
        Ok(match plan {
            Some(p) => self.client.plan_tasks(p).await?,
            None => self.client.my_planner_tasks().await?,
        })
    }

    pub(super) async fn src_buckets(&self, plan: &str) -> Res<Vec<Bucket>> {
        if self.apps_offline() {
            return Ok(teams_proto::planner::parse_buckets(&fx::planner_buckets(
                plan,
            )));
        }
        Ok(self.client.plan_buckets(plan).await?)
    }

    // -- Workflows --

    pub(super) async fn src_flow_environments(&self) -> Res<Vec<FlowEnvironment>> {
        if self.apps_offline() {
            return Ok(teams_proto::workflows::parse_environments(
                &fx::flow_environments(),
            ));
        }
        Ok(self.client.flow_environments().await?)
    }

    pub(super) async fn src_flows(&self, env: &str) -> Res<Vec<Flow>> {
        if self.apps_offline() {
            return Ok(teams_proto::workflows::parse_flows(&fx::flows(now_ms())));
        }
        Ok(self.client.flows(env).await?)
    }

    pub(super) async fn src_runs(&self, env: &str, flow: &str) -> Res<Vec<FlowRun>> {
        if self.apps_offline() {
            let v = fx::flow_runs(flow, now_ms());
            return Ok(teams_proto::workflows::parse_runs(&v));
        }
        Ok(self.client.flow_runs(env, flow, 20).await?)
    }

    // -- OneNote --

    pub(super) async fn src_notebooks(&self) -> Res<Vec<Notebook>> {
        if self.apps_offline() {
            let v = fx::notebooks(now_ms());
            return Ok(teams_proto::onenote::parse_notebooks(&v));
        }
        let mut out = self.client.my_notebooks().await?;
        for g in self.client.joined_group_ids().await.unwrap_or_default() {
            for n in self.client.group_notebooks(&g).await.unwrap_or_default() {
                if !out.iter().any(|o| o.id == n.id) {
                    out.push(n);
                }
            }
        }
        Ok(out)
    }

    pub(super) async fn src_sections(&self, nb: &Notebook) -> Res<Vec<Section>> {
        if self.apps_offline() {
            let v = fx::sections(now_ms());
            return Ok(teams_proto::onenote::parse_sections(&v));
        }
        Ok(self
            .client
            .notebook_sections(nb.sections_url.as_deref(), nb.section_groups_url.as_deref())
            .await?)
    }

    pub(super) async fn src_pages(&self, pages_url: &str) -> Res<Vec<Page>> {
        if self.apps_offline() {
            let v = fx::pages(pages_url, now_ms());
            return Ok(teams_proto::onenote::parse_pages(&v));
        }
        Ok(self.client.section_pages(pages_url).await?)
    }

    pub(super) async fn src_page_text(&self, content_url: &str) -> Res<String> {
        let html = if self.apps_offline() {
            fx::page_html(content_url)
        } else {
            self.client.page_html(content_url).await?
        };
        Ok(teams_proto::onenote::page_text(&html))
    }

    // -- Lists --

    pub(super) async fn src_sites(&self) -> Res<Vec<Site>> {
        if self.apps_offline() {
            return Ok(teams_proto::lists::parse_sites(&fx::sites()));
        }
        let mut sites = self.client.team_sites().await?;
        sites.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(sites)
    }

    pub(super) async fn src_site_lists(&self, site: &str) -> Res<Vec<SpList>> {
        if self.apps_offline() {
            return Ok(teams_proto::lists::parse_lists(&fx::site_lists(now_ms())));
        }
        Ok(self.client.site_lists(site).await?)
    }

    pub(super) async fn src_columns(&self, site: &str, list: &str) -> Res<Vec<Column>> {
        if self.apps_offline() {
            return Ok(teams_proto::lists::parse_columns(&fx::list_columns()));
        }
        Ok(self.client.list_columns(site, list).await?)
    }

    pub(super) async fn src_items(&self, site: &str, list: &str) -> Res<Vec<ListItem>> {
        if self.apps_offline() {
            return Ok(teams_proto::lists::parse_items(&fx::list_items()));
        }
        Ok(self.client.list_items(site, list, 1).await?)
    }

    // -- Meeting Recaps --

    /// The meetings of the last 30 days that have a recording or transcript, and where the list
    /// came from. The meeting content service first; when it fails or has nothing, the
    /// recordings in the user's OneDrive.
    pub(super) async fn src_recaps(&self) -> Res<(Vec<RecapMeeting>, &'static str)> {
        const MCPS: &str = "From the meeting content service.";
        if self.apps_offline() {
            let v = fx::recap_meetings(now_ms());
            return Ok((teams_proto::recaps::parse_meetings(&v), MCPS));
        }
        match self.client.recap_meetings(30).await {
            Ok(m) if !m.is_empty() => return Ok((m, MCPS)),
            Ok(_) => {}
            Err(e) => {
                tracing::info!(err = %e, "meeting content service failed, using OneDrive recordings")
            }
        }
        let files: Vec<RecordingFile> = onedrive::around::meeting_recordings(&self.client)
            .await?
            .into_iter()
            .map(|r| RecordingFile {
                id: r.item.id,
                drive_id: r.item.drive_id,
                name: r.item.name,
                web_url: r.item.web_url,
                modified_ms: r.item.modified_ms,
            })
            .collect();
        let cutoff = now_ms() - 30 * 86_400_000;
        let mut meetings = teams_proto::recaps::meetings_from_files(&files);
        meetings.retain(|m| m.start_ms.is_none_or(|s| s >= cutoff));
        Ok((meetings, "From the recordings in your OneDrive."))
    }

    /// The transcript of a meeting as text: a `.vtt` file of the Recordings folder, else the
    /// transcripts SharePoint keeps with the recording.
    pub(super) async fn src_transcript(
        &self,
        transcript: Option<&RecapResource>,
        recording: Option<&RecapResource>,
    ) -> Res<String> {
        use teams_proto::recaps::{parse_vtt, transcript_text};
        let vtt = if self.apps_offline() {
            fx::transcript_vtt().to_owned()
        } else if let Some(t) = transcript.filter(|t| t.name.to_lowercase().ends_with(".vtt")) {
            let (Some(d), Some(i)) = (&t.drive_id, &t.item_id) else {
                anyhow::bail!("the transcript file has no drive location");
            };
            self.client.drive_file_text(d, i).await?
        } else {
            let r = recording
                .or(transcript)
                .ok_or_else(|| anyhow::anyhow!("there is no recording or transcript"))?;
            let (Some(d), Some(i), Some(u)) = (&r.drive_id, &r.item_id, &r.web_url) else {
                anyhow::bail!("the recording has no SharePoint location");
            };
            self.client.sharepoint_transcript(u, d, i).await?
        };
        let text = transcript_text(&parse_vtt(&vtt));
        if text.is_empty() {
            anyhow::bail!("the transcript has no readable text");
        }
        Ok(text)
    }

    /// Where the meeting's notes tab opens, from the meeting chat's tabs.
    pub(super) async fn src_meeting_notes(&self, thread_id: &str) -> Res<Option<String>> {
        if thread_id.is_empty() {
            return Ok(None);
        }
        let tabs = if self.apps_offline() {
            teams_proto::channel_tabs::parse_tabs(&fx::meeting_tabs())
        } else {
            self.client.chat_tabs(thread_id).await?
        };
        Ok(teams_proto::recaps::notes_link(&tabs))
    }
}
