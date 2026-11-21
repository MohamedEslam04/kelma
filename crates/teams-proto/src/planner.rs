//! Planner and Microsoft To Do through Microsoft Graph: reads, and the writes behind New task,
//! Complete, Edit and Delete (`*_request` functions that build a [`Write`]).
//!
//! `GET /me/planner/tasks`, `/me/planner/plans`, `/groups/{id}/planner/plans`,
//! `/planner/plans/{id}/{tasks,buckets}`, `/me/todo/lists` and `/me/todo/lists/{id}/tasks`.
//! The `tasks.office.com` audience mints a token, but its API paths are undocumented and every
//! probe answered 404 or 503, so IronTeams does not use it. Premium plans are not reachable here.

use crate::client::Client;
use crate::error::Result;
use crate::graph::{Write, graph_url, ms_of, str_of, values};
use crate::time::iso_utc;
use reqwest::Method;
use serde_json::{Value, json};

/// A Planner plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub id: String,
    pub title: String,
    /// The Microsoft 365 group (team) that owns the plan.
    pub owner_group: Option<String>,
    pub created_ms: Option<i64>,
}

/// A column of a plan's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    pub id: String,
    pub name: String,
    pub plan_id: String,
}

/// A Planner task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerTask {
    pub id: String,
    pub plan_id: String,
    pub bucket_id: String,
    pub title: String,
    /// 0 to 100 (Planner uses 0, 50 and 100).
    pub percent_complete: u8,
    /// 0 to 10; 1 urgent, 3 important, 5 medium, 9 low.
    pub priority: u8,
    pub start_ms: Option<i64>,
    pub due_ms: Option<i64>,
    pub completed_ms: Option<i64>,
    /// User ids the task is assigned to.
    pub assignees: Vec<String>,
    pub checklist_items: u32,
    /// `@odata.etag`: Planner needs it as `If-Match` to change or delete the task.
    pub etag: Option<String>,
}

/// A To Do list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoList {
    pub id: String,
    pub name: String,
    pub is_shared: bool,
    /// `defaultList` or `flaggedEmails` for the built-in lists.
    pub well_known: Option<String>,
}

/// A To Do task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoTask {
    pub id: String,
    pub title: String,
    /// `notStarted`, `inProgress`, `completed`, `waitingOnOthers` or `deferred`.
    pub status: String,
    /// `low`, `normal` or `high`.
    pub importance: String,
    pub due_ms: Option<i64>,
    pub completed_ms: Option<i64>,
}

impl TodoTask {
    pub fn is_completed(&self) -> bool {
        self.status == "completed"
    }
}

/// Plans from a Graph collection.
pub fn parse_plans(v: &Value) -> Vec<Plan> {
    values(v).iter().filter_map(parse_plan).collect()
}

fn parse_plan(p: &Value) -> Option<Plan> {
    Some(Plan {
        id: str_of(p, "id")?,
        title: str_of(p, "title").unwrap_or_default(),
        owner_group: str_of(p, "owner"),
        created_ms: ms_of(p, "createdDateTime"),
    })
}

/// Buckets from a Graph collection.
pub fn parse_buckets(v: &Value) -> Vec<Bucket> {
    values(v)
        .iter()
        .filter_map(|b| {
            Some(Bucket {
                id: str_of(b, "id")?,
                name: str_of(b, "name").unwrap_or_default(),
                plan_id: str_of(b, "planId").unwrap_or_default(),
            })
        })
        .collect()
}

/// Planner tasks from a Graph collection.
pub fn parse_planner_tasks(v: &Value) -> Vec<PlannerTask> {
    values(v).iter().filter_map(parse_planner_task).collect()
}

fn parse_planner_task(t: &Value) -> Option<PlannerTask> {
    let num = |k: &str| t[k].as_u64().unwrap_or(0);
    Some(PlannerTask {
        id: str_of(t, "id")?,
        plan_id: str_of(t, "planId").unwrap_or_default(),
        bucket_id: str_of(t, "bucketId").unwrap_or_default(),
        title: str_of(t, "title").unwrap_or_default(),
        percent_complete: num("percentComplete").min(100) as u8,
        priority: num("priority").min(10) as u8,
        start_ms: ms_of(t, "startDateTime"),
        due_ms: ms_of(t, "dueDateTime"),
        completed_ms: ms_of(t, "completedDateTime"),
        assignees: t["assignments"]
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default(),
        checklist_items: num("checklistItemCount") as u32,
        etag: str_of(t, "@odata.etag"),
    })
}

/// To Do lists from a Graph collection.
pub fn parse_todo_lists(v: &Value) -> Vec<TodoList> {
    values(v)
        .iter()
        .filter_map(|l| {
            Some(TodoList {
                id: str_of(l, "id")?,
                name: str_of(l, "displayName").unwrap_or_default(),
                is_shared: l["isShared"].as_bool().unwrap_or(false),
                well_known: str_of(l, "wellknownListName").filter(|w| w != "none"),
            })
        })
        .collect()
}

/// To Do tasks from a Graph collection.
pub fn parse_todo_tasks(v: &Value) -> Vec<TodoTask> {
    values(v)
        .iter()
        .filter_map(|t| {
            Some(TodoTask {
                id: str_of(t, "id")?,
                title: str_of(t, "title").unwrap_or_default(),
                status: str_of(t, "status").unwrap_or_default(),
                importance: str_of(t, "importance").unwrap_or_default(),
                due_ms: ms_of(t, "dueDateTime"),
                completed_ms: ms_of(t, "completedDateTime"),
            })
        })
        .collect()
}

/// What an edit changes. `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskPatch {
    pub title: Option<String>,
    /// `Some(None)` clears the due date.
    pub due: Option<Option<i64>>,
    pub done: Option<bool>,
}

/// A To Do date: `dateTimeTimeZone` at 00:00 UTC.
fn todo_date(ms: i64) -> Value {
    json!({"dateTime": iso_utc(ms).replace('Z', ".0000000"), "timeZone": "UTC"})
}

/// A new task in a To Do list: `POST /me/todo/lists/{list}/tasks`.
pub fn todo_create_request(list_id: &str, title: &str, due_ms: Option<i64>) -> Write {
    let mut body = json!({"title": title});
    if let Some(d) = due_ms {
        body["dueDateTime"] = todo_date(d);
    }
    Write::graph(
        Method::POST,
        graph_url(&["me", "todo", "lists", list_id, "tasks"], ""),
        Some(body),
    )
}

/// An edit of a To Do task: `PATCH /me/todo/lists/{list}/tasks/{task}`. Completing sets
/// `status` to `completed`; undoing it goes back to `notStarted`.
pub fn todo_update_request(list_id: &str, task_id: &str, p: &TaskPatch) -> Write {
    let mut body = json!({});
    if let Some(t) = &p.title {
        body["title"] = json!(t);
    }
    match p.due {
        Some(Some(d)) => body["dueDateTime"] = todo_date(d),
        Some(None) => body["dueDateTime"] = Value::Null,
        None => {}
    }
    if let Some(done) = p.done {
        body["status"] = json!(if done { "completed" } else { "notStarted" });
    }
    Write::graph(
        Method::PATCH,
        graph_url(&["me", "todo", "lists", list_id, "tasks", task_id], ""),
        Some(body),
    )
}

/// `DELETE /me/todo/lists/{list}/tasks/{task}`.
pub fn todo_delete_request(list_id: &str, task_id: &str) -> Write {
    Write::graph(
        Method::DELETE,
        graph_url(&["me", "todo", "lists", list_id, "tasks", task_id], ""),
        None,
    )
}

/// A new Planner task: `POST /planner/tasks` with the plan and, when known, the bucket.
pub fn planner_create_request(
    plan_id: &str,
    bucket_id: Option<&str>,
    title: &str,
    due_ms: Option<i64>,
) -> Write {
    let mut body = json!({"planId": plan_id, "title": title});
    if let Some(b) = bucket_id {
        body["bucketId"] = json!(b);
    }
    if let Some(d) = due_ms {
        body["dueDateTime"] = json!(iso_utc(d));
    }
    Write::graph(
        Method::POST,
        graph_url(&["planner", "tasks"], ""),
        Some(body),
    )
}

/// An edit of a Planner task: `PATCH /planner/tasks/{task}` with `If-Match`. Completing sets
/// `percentComplete` to 100, undoing it to 0. `None` without an etag: Planner refuses the edit.
pub fn planner_update_request(task_id: &str, etag: Option<&str>, p: &TaskPatch) -> Option<Write> {
    let mut body = json!({});
    if let Some(t) = &p.title {
        body["title"] = json!(t);
    }
    match p.due {
        Some(Some(d)) => body["dueDateTime"] = json!(iso_utc(d)),
        Some(None) => body["dueDateTime"] = Value::Null,
        None => {}
    }
    if let Some(done) = p.done {
        body["percentComplete"] = json!(if done { 100 } else { 0 });
    }
    let w = Write::graph(
        Method::PATCH,
        graph_url(&["planner", "tasks", task_id], ""),
        Some(body),
    );
    Some(w.if_match(etag?))
}

/// `DELETE /planner/tasks/{task}` with `If-Match`; `None` without an etag.
pub fn planner_delete_request(task_id: &str, etag: Option<&str>) -> Option<Write> {
    let w = Write::graph(
        Method::DELETE,
        graph_url(&["planner", "tasks", task_id], ""),
        None,
    );
    Some(w.if_match(etag?))
}

/// Wraps already-collected items so the `parse_*` functions can read them.
fn collection(items: Vec<Value>) -> Value {
    serde_json::json!({ "value": items })
}

const PAGES: usize = 10;

impl Client {
    /// The signed-in user's Planner tasks, across all plans.
    pub async fn my_planner_tasks(&self) -> Result<Vec<PlannerTask>> {
        let url = graph_url(&["me", "planner", "tasks"], "");
        let items = self.graph_values("planner tasks", &url, PAGES).await?;
        Ok(parse_planner_tasks(&collection(items)))
    }

    /// Plans the user created or was added to.
    pub async fn my_planner_plans(&self) -> Result<Vec<Plan>> {
        let url = graph_url(&["me", "planner", "plans"], "");
        let items = self.graph_values("planner plans", &url, PAGES).await?;
        Ok(parse_plans(&collection(items)))
    }

    /// The plans of a team's group.
    pub async fn group_planner_plans(&self, group_id: &str) -> Result<Vec<Plan>> {
        let url = graph_url(&["groups", group_id, "planner", "plans"], "");
        let items = self.graph_values("group plans", &url, PAGES).await?;
        Ok(parse_plans(&collection(items)))
    }

    /// The tasks of one plan.
    pub async fn plan_tasks(&self, plan_id: &str) -> Result<Vec<PlannerTask>> {
        let url = graph_url(&["planner", "plans", plan_id, "tasks"], "");
        let items = self.graph_values("plan tasks", &url, PAGES).await?;
        Ok(parse_planner_tasks(&collection(items)))
    }

    /// The buckets (board columns) of one plan.
    pub async fn plan_buckets(&self, plan_id: &str) -> Result<Vec<Bucket>> {
        let url = graph_url(&["planner", "plans", plan_id, "buckets"], "");
        let items = self.graph_values("plan buckets", &url, PAGES).await?;
        Ok(parse_buckets(&collection(items)))
    }

    /// The user's To Do lists.
    pub async fn todo_lists(&self) -> Result<Vec<TodoList>> {
        let url = graph_url(&["me", "todo", "lists"], "");
        let items = self.graph_values("todo lists", &url, PAGES).await?;
        Ok(parse_todo_lists(&collection(items)))
    }

    /// The tasks of one To Do list.
    pub async fn todo_tasks(&self, list_id: &str) -> Result<Vec<TodoTask>> {
        let url = graph_url(&["me", "todo", "lists", list_id, "tasks"], "");
        let items = self.graph_values("todo tasks", &url, PAGES).await?;
        Ok(parse_todo_tasks(&collection(items)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_planner_tasks() {
        let v = json!({"value":[
            {"@odata.etag":"W/\"abc\"","id":"t1","planId":"p1","bucketId":"b1","title":"Write spec","percentComplete":50,
             "priority":3,"startDateTime":"2026-01-02T00:00:00Z","dueDateTime":"2026-01-05T00:00:00Z",
             "completedDateTime":null,"checklistItemCount":4,
             "assignments":{"user-a":{"orderHint":"x"},"user-b":{}}},
            {"id":"t2","title":"Bare"},
            {"title":"no id is skipped"}
        ]});
        let tasks = parse_planner_tasks(&v);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Write spec");
        assert_eq!(tasks[0].percent_complete, 50);
        assert_eq!(tasks[0].priority, 3);
        assert_eq!(tasks[0].due_ms, Some(1_767_571_200_000));
        assert_eq!(tasks[0].completed_ms, None);
        assert_eq!(tasks[0].checklist_items, 4);
        assert_eq!(tasks[0].etag.as_deref(), Some("W/\"abc\""));
        assert_eq!(tasks[1].etag, None);
        let mut who = tasks[0].assignees.clone();
        who.sort();
        assert_eq!(who, ["user-a", "user-b"]);
        assert_eq!(tasks[1].percent_complete, 0);
        assert!(tasks[1].assignees.is_empty());
    }

    #[test]
    fn to_do_writes_use_the_graph_task_shapes() {
        let due = 1_795_564_800_000; // 2026-11-25
        let w = todo_create_request("L 1", "Buy milk", Some(due));
        assert_eq!(w.method, Method::POST);
        assert_eq!(
            w.url,
            "https://graph.microsoft.com/v1.0/me/todo/lists/L%201/tasks"
        );
        assert_eq!(
            w.body,
            crate::graph::WriteBody::Json(json!({"title": "Buy milk", "dueDateTime":
                {"dateTime": "2026-11-25T00:00:00.0000000", "timeZone": "UTC"}}))
        );
        let no_due = todo_create_request("l", "x", None);
        assert_eq!(
            no_due.body,
            crate::graph::WriteBody::Json(json!({"title": "x"}))
        );

        let done = todo_update_request(
            "l",
            "t",
            &TaskPatch {
                done: Some(true),
                ..Default::default()
            },
        );
        assert_eq!(done.method, Method::PATCH);
        assert!(done.url.ends_with("/me/todo/lists/l/tasks/t"));
        assert_eq!(
            done.body,
            crate::graph::WriteBody::Json(json!({"status": "completed"}))
        );
        let edit = todo_update_request(
            "l",
            "t",
            &TaskPatch {
                title: Some("New".into()),
                due: Some(None),
                done: Some(false),
            },
        );
        assert_eq!(
            edit.body,
            crate::graph::WriteBody::Json(
                json!({"title": "New", "dueDateTime": null, "status": "notStarted"})
            )
        );
        let del = todo_delete_request("l", "t");
        assert_eq!(del.method, Method::DELETE);
        assert_eq!(del.body, crate::graph::WriteBody::None);
    }

    #[test]
    fn planner_writes_carry_the_etag_and_the_plan() {
        let w = planner_create_request("p1", Some("b1"), "Write spec", Some(1_795_564_800_000));
        assert_eq!(w.url, "https://graph.microsoft.com/v1.0/planner/tasks");
        assert_eq!(
            w.body,
            crate::graph::WriteBody::Json(json!({"planId": "p1", "bucketId": "b1",
                "title": "Write spec", "dueDateTime": "2026-11-25T00:00:00Z"}))
        );
        assert!(w.headers.is_empty());
        let patch = TaskPatch {
            done: Some(true),
            ..Default::default()
        };
        let u = planner_update_request("t1", Some("W/\"x\""), &patch).unwrap();
        assert_eq!(u.method, Method::PATCH);
        assert_eq!(u.headers, [("If-Match", "W/\"x\"".to_owned())]);
        assert_eq!(
            u.body,
            crate::graph::WriteBody::Json(json!({"percentComplete": 100}))
        );
        assert!(
            planner_update_request("t1", None, &patch).is_none(),
            "no etag, no edit"
        );
        let d = planner_delete_request("t1", Some("e")).unwrap();
        assert_eq!(d.method, Method::DELETE);
        assert_eq!(d.headers[0].0, "If-Match");
        assert!(planner_delete_request("t1", None).is_none());
    }

    #[test]
    fn parses_plans_and_buckets() {
        let plans = parse_plans(&json!({"value":[
            {"id":"p1","title":"Launch","owner":"g1","createdDateTime":"2026-01-01T00:00:00Z"}]}));
        assert_eq!(plans[0].title, "Launch");
        assert_eq!(plans[0].owner_group.as_deref(), Some("g1"));
        assert_eq!(plans[0].created_ms, Some(1_767_225_600_000));
        let buckets = parse_buckets(&json!({"value":[{"id":"b1","name":"To do","planId":"p1"}]}));
        assert_eq!(buckets[0].name, "To do");
        assert_eq!(buckets[0].plan_id, "p1");
    }

    #[test]
    fn parses_todo_lists_and_tasks() {
        let lists = parse_todo_lists(&json!({"value":[
            {"id":"l1","displayName":"Tasks","isShared":false,"wellknownListName":"defaultList"},
            {"id":"l2","displayName":"Groceries","isShared":true,"wellknownListName":"none"}]}));
        assert_eq!(lists[0].well_known.as_deref(), Some("defaultList"));
        assert_eq!(lists[1].well_known, None);
        assert!(lists[1].is_shared);
        let tasks = parse_todo_tasks(&json!({"value":[
            {"id":"a","title":"Buy milk","status":"notStarted","importance":"high",
             "dueDateTime":{"dateTime":"2026-01-02T00:00:00.0000000","timeZone":"UTC"}},
            {"id":"b","title":"Done","status":"completed",
             "completedDateTime":{"dateTime":"2026-01-01T10:00:00.0000000","timeZone":"UTC"}}]}));
        assert_eq!(tasks[0].importance, "high");
        assert_eq!(tasks[0].due_ms, Some(1_767_312_000_000));
        assert!(!tasks[0].is_completed());
        assert!(tasks[1].is_completed());
        assert_eq!(tasks[1].completed_ms, Some(1_767_261_600_000));
    }
}
