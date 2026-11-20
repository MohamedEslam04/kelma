//! Planner and To Do: My tasks, plans with their board, and To Do lists, with New task,
//! Complete, Edit and Delete (Graph `/me/todo/lists/{id}/tasks` and `/planner/tasks`).

use super::form::{self, Pending, field};
use super::view::{Column, Row, TasksData, day};
use crate::backend::Engine;
use std::sync::Arc;
use teams_proto::graph::Write;
use teams_proto::planner::{
    Bucket, Plan, PlannerTask, TaskPatch, TodoList, TodoTask, planner_create_request,
    planner_delete_request, planner_update_request, todo_create_request, todo_delete_request,
    todo_update_request,
};
use teams_proto::time::{iso_utc, now_ms, parse_date_input};

#[derive(Default)]
pub struct Tasks {
    loaded: bool,
    lists: Vec<TodoList>,
    plans: Vec<Plan>,
    /// `my`, `plan:<id>` or `todo:<id>`.
    nav: String,
    board: bool,
    todo: Vec<TodoTask>,
    planner: Vec<PlannerTask>,
    buckets: Vec<Bucket>,
    status: String,
    loading: bool,
}

const NOT_STARTED: &str = "Not started";
const IN_PROGRESS: &str = "In progress";
const COMPLETED: &str = "Completed";

fn priority(p: u8) -> &'static str {
    match p {
        0 => "",
        1 => "Urgent",
        2..=4 => "Important",
        5..=7 => "Medium",
        _ => "Low",
    }
}

fn todo_status(s: &str) -> &'static str {
    match s {
        "completed" => COMPLETED,
        "inProgress" => IN_PROGRESS,
        "waitingOnOthers" => "Waiting on others",
        "deferred" => "Deferred",
        _ => NOT_STARTED,
    }
}

fn planner_status(pct: u8) -> &'static str {
    match pct {
        0 => NOT_STARTED,
        100 => COMPLETED,
        _ => IN_PROGRESS,
    }
}

fn list_name(l: &TodoList) -> String {
    match l.well_known.as_deref() {
        Some("defaultList") => "Tasks".into(),
        Some("flaggedEmails") => "Flagged email".into(),
        _ => l.name.clone(),
    }
}

/// `Due Nov 20`, with `overdue` when it is past and the task is open.
fn due_text(due_ms: Option<i64>, done: bool, now: i64) -> String {
    match due_ms {
        None => String::new(),
        Some(d) if !done && d < now => format!("Due {} (overdue)", day(d)),
        Some(d) => format!("Due {}", day(d)),
    }
}

fn tone(status: &str) -> &'static str {
    match status {
        COMPLETED => "ok",
        IN_PROGRESS => "warn",
        _ => "",
    }
}

/// A due date as the dialog types it (`2026-11-25`), empty for none.
fn due_input(ms: Option<i64>) -> String {
    ms.map(|m| iso_utc(m)[..10].to_owned()).unwrap_or_default()
}

/// The typed due date: none when empty, else a real `YYYY-MM-DD` date.
fn parse_due(text: &str) -> Result<Option<i64>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    parse_date_input(text)
        .map(Some)
        .ok_or_else(|| "The due date must look like 2026-11-25.".to_owned())
}

impl Tasks {
    fn todo_list(&self) -> Option<&str> {
        self.nav.strip_prefix("todo:")
    }

    fn plan(&self) -> Option<&str> {
        self.nav.strip_prefix("plan:")
    }

    /// A task's title, due date and whether it is done.
    fn task(&self, id: &str) -> Option<(String, Option<i64>, bool)> {
        if self.todo_list().is_some() {
            let t = self.todo.iter().find(|t| t.id == id)?;
            return Some((t.title.clone(), t.due_ms, t.is_completed()));
        }
        let t = self.planner.iter().find(|t| t.id == id)?;
        Some((t.title.clone(), t.due_ms, t.percent_complete >= 100))
    }

    fn etag(&self, id: &str) -> Option<&str> {
        self.planner
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.etag.as_deref())
    }

    /// The edit request for the open list or plan. Planner needs the task's etag.
    fn update_write(&self, id: &str, p: &TaskPatch) -> Result<Write, String> {
        match self.todo_list() {
            Some(list) => Ok(todo_update_request(list, id, p)),
            None => planner_update_request(id, self.etag(id), p).ok_or_else(|| {
                "Planner needs a fresh copy of this task. Refresh and try again.".into()
            }),
        }
    }

    fn nav_rows(&self) -> Vec<Row> {
        let header = |t: &str| Row {
            title: t.into(),
            header: true,
            ..Default::default()
        };
        let item = |id: String, title: String, sel: &str| Row {
            selected: sel == id,
            id,
            title,
            ..Default::default()
        };
        let mut rows = vec![
            header("Planner"),
            item("my".into(), "My tasks".into(), &self.nav),
        ];
        rows.extend(
            self.plans
                .iter()
                .map(|p| item(format!("plan:{}", p.id), p.title.clone(), &self.nav)),
        );
        rows.push(header("To Do"));
        rows.extend(
            self.lists
                .iter()
                .map(|l| item(format!("todo:{}", l.id), list_name(l), &self.nav)),
        );
        rows
    }

    fn title(&self) -> String {
        self.nav_rows()
            .into_iter()
            .find(|r| r.selected)
            .map(|r| r.title)
            .unwrap_or_default()
    }

    /// Every open task as a row, whichever service it came from.
    fn rows(&self, now: i64) -> Vec<Row> {
        if self.nav.starts_with("todo:") {
            return self
                .todo
                .iter()
                .map(|t| {
                    let status = todo_status(&t.status);
                    Row {
                        id: t.id.clone(),
                        title: t.title.clone(),
                        subtitle: due_text(t.due_ms, t.is_completed(), now),
                        meta: match t.importance.as_str() {
                            "high" => "High importance".into(),
                            "low" => "Low importance".into(),
                            _ => String::new(),
                        },
                        badge: status.into(),
                        tone: tone(status),
                        done: t.is_completed(),
                        ..Default::default()
                    }
                })
                .collect();
        }
        self.planner
            .iter()
            .map(|t| {
                let status = planner_status(t.percent_complete);
                let done = t.percent_complete >= 100;
                let place = if self.nav == "my" {
                    self.plans
                        .iter()
                        .find(|p| p.id == t.plan_id)
                        .map(|p| p.title.clone())
                } else {
                    self.buckets
                        .iter()
                        .find(|b| b.id == t.bucket_id)
                        .map(|b| b.name.clone())
                };
                let meta: Vec<String> = [Some(priority(t.priority).to_owned()), place]
                    .into_iter()
                    .flatten()
                    .filter(|m| !m.is_empty())
                    .collect();
                Row {
                    id: t.id.clone(),
                    title: t.title.clone(),
                    subtitle: due_text(t.due_ms, done, now),
                    meta: meta.join(" · "),
                    badge: status.into(),
                    tone: tone(status),
                    done,
                    ..Default::default()
                }
            })
            .collect()
    }

    /// The board: a column per bucket of a plan, else one per progress state.
    fn columns(&self, rows: &[Row]) -> Vec<Column> {
        if self.nav.starts_with("plan:") && !self.buckets.is_empty() {
            return self
                .buckets
                .iter()
                .map(|b| Column {
                    title: b.name.clone(),
                    cards: self
                        .planner
                        .iter()
                        .zip(rows)
                        .filter(|(t, _)| t.bucket_id == b.id)
                        .map(|(_, r)| r.clone())
                        .collect(),
                })
                .collect();
        }
        [NOT_STARTED, IN_PROGRESS, COMPLETED]
            .into_iter()
            .map(|s| Column {
                title: s.into(),
                cards: rows
                    .iter()
                    .filter(|r| {
                        r.badge == s
                            || (s == IN_PROGRESS
                                && !matches!(r.badge.as_str(), NOT_STARTED | COMPLETED))
                    })
                    .cloned()
                    .collect(),
            })
            .collect()
    }

    pub fn data(&self, now: i64) -> TasksData {
        let rows = self.rows(now);
        let count = |s: &str| rows.iter().filter(|r| r.badge == s).count();
        let (open, doing, done) = (count(NOT_STARTED), count(IN_PROGRESS), count(COMPLETED));
        let summary = if rows.is_empty() {
            String::new()
        } else {
            format!(
                "{} tasks: {open} not started, {doing} in progress, {done} completed",
                rows.len()
            )
        };
        let progress = if rows.is_empty() {
            0.0
        } else {
            done as f32 / rows.len() as f32
        };
        TasksData {
            nav: self.nav_rows(),
            columns: self.columns(&rows),
            title: self.title(),
            board: self.board,
            can_board: !rows.is_empty(),
            can_add: self.todo_list().is_some() || self.plan().is_some(),
            summary,
            progress,
            status: if self.status.is_empty() && !self.loading && rows.is_empty() && self.loaded {
                "No tasks here.".into()
            } else {
                self.status.clone()
            },
            loading: self.loading,
            tasks: rows,
        }
    }
}

impl Engine {
    fn tasks_push(self: &Arc<Self>) {
        let data = self.apps.lock().unwrap().tasks.data(now_ms());
        self.ui.apps_tasks(data);
    }

    fn tasks_edit(self: &Arc<Self>, f: impl FnOnce(&mut Tasks)) {
        f(&mut self.apps.lock().unwrap().tasks);
        self.tasks_push();
    }

    pub(super) async fn tasks_enter(self: &Arc<Self>) -> anyhow::Result<()> {
        self.ui.open_view("planner".into());
        let busy = {
            let t = &self.apps.lock().unwrap().tasks;
            t.loaded || t.loading
        };
        if busy {
            self.tasks_push();
            return Ok(());
        }
        self.tasks_refresh().await
    }

    pub(super) async fn tasks_refresh(self: &Arc<Self>) -> anyhow::Result<()> {
        self.tasks_edit(|t| {
            t.loading = true;
            t.status.clear();
        });
        let (lists, plans) = tokio::join!(self.src_todo_lists(), self.src_plans());
        let mut problems = Vec::new();
        let lists = lists.unwrap_or_else(|e| {
            problems.push(format!("To Do: {e}"));
            Vec::new()
        });
        let plans = plans.unwrap_or_else(|e| {
            problems.push(format!("Planner: {e}"));
            Vec::new()
        });
        let nav = {
            let mut a = self.apps.lock().unwrap();
            let t = &mut a.tasks;
            t.loaded = true;
            t.lists = lists;
            t.plans = plans;
            if t.nav.is_empty() || !t.nav_rows().iter().any(|r| r.id == t.nav) {
                t.nav = t
                    .lists
                    .iter()
                    .find(|l| l.well_known.as_deref() == Some("defaultList"))
                    .or(t.lists.first())
                    .map_or_else(|| "my".into(), |l| format!("todo:{}", l.id));
            }
            t.status = problems.join(". ");
            t.nav.clone()
        };
        self.tasks_nav(&nav).await
    }

    pub(super) async fn tasks_nav(self: &Arc<Self>, key: &str) -> anyhow::Result<()> {
        self.tasks_edit(|t| {
            t.nav = key.to_owned();
            t.loading = true;
            t.todo.clear();
            t.planner.clear();
            t.buckets.clear();
        });
        // A list loaded for a place the user already left (another list was picked meanwhile)
        // is dropped: it would show under the new place's name.
        let here = |a: &super::Apps| a.tasks.nav == key;
        let result: anyhow::Result<()> = async {
            if let Some(list) = key.strip_prefix("todo:") {
                let tasks = self.src_todo_tasks(list).await?;
                let mut a = self.apps.lock().unwrap();
                if here(&a) {
                    a.tasks.todo = tasks;
                }
            } else if let Some(plan) = key.strip_prefix("plan:") {
                let (tasks, buckets) =
                    tokio::join!(self.src_planner_tasks(Some(plan)), self.src_buckets(plan));
                let mut a = self.apps.lock().unwrap();
                if here(&a) {
                    a.tasks.planner = tasks?;
                    a.tasks.buckets = buckets.unwrap_or_default();
                }
            } else {
                let tasks = self.src_planner_tasks(None).await?;
                let mut a = self.apps.lock().unwrap();
                if here(&a) {
                    a.tasks.planner = tasks;
                }
            }
            Ok(())
        }
        .await;
        if self.apps.lock().unwrap().tasks.nav != key {
            return Ok(());
        }
        self.tasks_edit(|t| {
            t.loading = false;
            if let Err(e) = result {
                t.status = format!("Loading tasks failed: {e}");
            }
        });
        Ok(())
    }

    pub(super) fn tasks_mode(self: &Arc<Self>, mode: &str) -> anyhow::Result<()> {
        self.tasks_edit(|t| t.board = mode == "board");
        Ok(())
    }

    /// Shows what a write did: the banner, then the open list again from the service.
    async fn tasks_sent(
        self: &Arc<Self>,
        w: Write,
        done: &str,
        failed: &str,
    ) -> Result<(), String> {
        match self.src_write("task change", w).await {
            Ok(()) => {
                self.ui.banner(done.to_owned());
                let nav = self.apps.lock().unwrap().tasks.nav.clone();
                let _ = self.tasks_nav(&nav).await;
            }
            Err(err) => {
                tracing::warn!(%err, "{failed}");
                self.ui.banner(format!("{failed}: {err}"));
            }
        }
        Ok(())
    }

    /// Opens the New task dialog for the open To Do list or plan.
    pub(super) fn tasks_begin_new(self: &Arc<Self>) -> anyhow::Result<()> {
        let (title, ok) = {
            let a = self.apps.lock().unwrap();
            let t = &a.tasks;
            (t.title(), t.todo_list().is_some() || t.plan().is_some())
        };
        if !ok {
            self.ui
                .banner("Open a To Do list or a plan to add a task.".into());
            return Ok(());
        }
        let mut spec = form::confirm(format!("New task in {title}"), String::new(), "Add", false);
        spec.fields = vec![
            ("Title".into(), String::new()),
            ("Due date (2026-11-25, optional)".into(), String::new()),
        ];
        self.form_open(Pending::TaskNew, spec, String::new());
        Ok(())
    }

    pub(super) fn tasks_begin_edit(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let task = self.apps.lock().unwrap().tasks.task(id);
        let Some((title, due, _)) = task else {
            return Ok(());
        };
        let mut spec = form::confirm("Edit task".into(), String::new(), "Save", false);
        spec.fields = vec![
            ("Title".into(), title),
            (
                "Due date (2026-11-25, empty for none)".into(),
                due_input(due),
            ),
        ];
        self.form_open(Pending::TaskEdit(id.to_owned()), spec, String::new());
        Ok(())
    }

    pub(super) fn tasks_begin_delete(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let task = self.apps.lock().unwrap().tasks.task(id);
        let Some((title, _, _)) = task else {
            return Ok(());
        };
        let spec = form::confirm(
            "Delete this task?".into(),
            format!("\"{title}\" is deleted. This cannot be undone."),
            "Delete",
            true,
        );
        self.form_open(Pending::TaskDelete(id.to_owned()), spec, String::new());
        Ok(())
    }

    /// Complete, or undo it. No dialog: it is one click to undo.
    pub(super) async fn tasks_toggle(self: &Arc<Self>, id: &str) -> anyhow::Result<()> {
        let (state, write) = {
            let a = self.apps.lock().unwrap();
            let Some((_, _, done)) = a.tasks.task(id) else {
                return Ok(());
            };
            let patch = TaskPatch {
                done: Some(!done),
                ..Default::default()
            };
            (done, a.tasks.update_write(id, &patch))
        };
        match write {
            Ok(w) => {
                let (ok, bad) = if state {
                    ("Marked as not completed.", "Could not change the task")
                } else {
                    ("Task completed.", "Could not complete the task")
                };
                let _ = self.tasks_sent(w, ok, bad).await;
            }
            Err(msg) => self.ui.banner(msg),
        }
        Ok(())
    }

    pub(super) async fn tasks_do_new(self: &Arc<Self>, fields: &[String]) -> Result<(), String> {
        let title = field(fields, 0);
        if title.is_empty() {
            return Err("Give the task a title.".into());
        }
        let due = parse_due(&field(fields, 1))?;
        let w = {
            let a = self.apps.lock().unwrap();
            let t = &a.tasks;
            match (t.todo_list(), t.plan()) {
                (Some(list), _) => todo_create_request(list, &title, due),
                (None, Some(plan)) => {
                    let bucket = t.buckets.first().map(|b| b.id.as_str());
                    planner_create_request(plan, bucket, &title, due)
                }
                _ => return Err("Open a To Do list or a plan to add a task.".into()),
            }
        };
        self.tasks_sent(w, "Task added.", "Could not add the task")
            .await
    }

    pub(super) async fn tasks_do_edit(
        self: &Arc<Self>,
        id: &str,
        fields: &[String],
    ) -> Result<(), String> {
        let title = field(fields, 0);
        if title.is_empty() {
            return Err("The title cannot be empty.".into());
        }
        let due = parse_due(&field(fields, 1))?;
        let patch = TaskPatch {
            title: Some(title),
            due: Some(due),
            done: None,
        };
        let w = self.apps.lock().unwrap().tasks.update_write(id, &patch)?;
        self.tasks_sent(w, "Task saved.", "Could not save the task")
            .await
    }

    pub(super) async fn tasks_do_delete(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let w = {
            let a = self.apps.lock().unwrap();
            match a.tasks.todo_list() {
                Some(list) => todo_delete_request(list, id),
                None => planner_delete_request(id, a.tasks.etag(id)).ok_or_else(|| {
                    "Planner needs a fresh copy of this task. Refresh and try again.".to_owned()
                })?,
            }
        };
        self.tasks_sent(w, "Task deleted.", "Could not delete the task")
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teams_proto::planner::{parse_buckets, parse_planner_tasks, parse_plans};

    #[test]
    fn plan_board_has_a_column_per_bucket_and_my_tasks_group_by_progress() {
        let now = now_ms();
        let plans = parse_plans(&super::super::fixtures::planner_plans());
        let mut t = Tasks {
            loaded: true,
            nav: "plan:plan-1".into(),
            plans,
            planner: parse_planner_tasks(&super::super::fixtures::planner_tasks(
                Some("plan-1"),
                now,
            )),
            buckets: parse_buckets(&super::super::fixtures::planner_buckets("plan-1")),
            ..Default::default()
        };
        let d = t.data(now);
        let cols: Vec<(&str, usize)> = d
            .columns
            .iter()
            .map(|c| (c.title.as_str(), c.cards.len()))
            .collect();
        assert_eq!(cols, [("To do", 1), ("Doing", 1), ("Done", 1)]);
        assert_eq!(
            d.summary,
            "3 tasks: 1 not started, 1 in progress, 1 completed"
        );
        assert!((d.progress - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(d.tasks[0].meta, "Important · To do");

        t.nav = "my".into();
        let d = t.data(now);
        let cols: Vec<&str> = d.columns.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(cols, ["Not started", "In progress", "Completed"]);
        // On My tasks the place is the plan, not the bucket.
        assert!(d.tasks[0].meta.ends_with("Sprint board"));
    }

    #[test]
    fn overdue_open_tasks_say_so() {
        let now = 10 * 86_400_000;
        assert!(due_text(Some(now - 86_400_000), false, now).ends_with("(overdue)"));
        assert!(!due_text(Some(now - 86_400_000), true, now).contains("overdue"));
        assert_eq!(due_text(None, false, now), "");
    }
}
