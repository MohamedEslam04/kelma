//! Demo data for the app screens: synthetic answers in the shape the services send, so demo
//! mode and the tests run the same parsers and view builders as the live client. Nothing here
//! comes from a real tenant.

use serde_json::{json, Value};

const MS_DAY: i64 = 24 * 3600 * 1000;

fn iso(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

pub const PLANNER: &str = "com.microsoft.teamspace.tab.planner";
pub const ONENOTE: &str = "demo-onenote";
pub const WORKFLOWS: &str = "demo-workflows";
pub const LISTS: &str = "demo-lists";
pub const SHIFTS: &str = "demo-shifts";
pub const RECAPS: &str = "demo-recaps";
pub const POLLS: &str = "demo-polls";
pub const AGENT: &str = "demo-agent";

/// One app of the demo catalog.
struct App {
    id: &'static str,
    name: &'static str,
    developer: &'static str,
    blurb: &'static str,
    accent: &'static str,
    /// `Personal`, `Team`, `groupchat`.
    scopes: &'static [&'static str],
    core: bool,
    agent: bool,
    paid: bool,
    version: &'static str,
    /// Device permissions the manifest asks for.
    device: &'static [&'static str],
}

const APPS: &[App] = &[
    app(
        "demo-activity",
        "Activity",
        "Microsoft Corporation",
        "",
        "",
        &["Personal"],
        true,
        "1.0",
    ),
    app(
        "demo-chat",
        "Chat",
        "Microsoft Corporation",
        "",
        "",
        &["Personal"],
        true,
        "1.0",
    ),
    app(
        "demo-teams",
        "Teams",
        "Microsoft Corporation",
        "",
        "",
        &["Personal"],
        true,
        "1.0",
    ),
    app(
        "demo-calendar",
        "Calendar",
        "Microsoft Corporation",
        "",
        "",
        &["Personal"],
        true,
        "1.0",
    ),
    app(
        "demo-calling",
        "Calling",
        "Microsoft Corporation",
        "Calls and voicemail",
        "",
        &["Personal"],
        true,
        "1.0",
    ),
    app(
        "demo-onedrive",
        "OneDrive",
        "Microsoft Corporation",
        "",
        "",
        &["Personal"],
        true,
        "1.0",
    ),
    App {
        device: &[],
        ..app(
            PLANNER,
            "Planner",
            "Microsoft Corporation",
            "A simple way to organize tasks and projects",
            "#2b579a",
            &["Personal", "Team"],
            false,
            "1.9.0",
        )
    },
    App {
        device: &["media"],
        ..app(
            ONENOTE,
            "OneNote",
            "Microsoft Corporation",
            "Notes that follow you everywhere",
            "#7719aa",
            &["Personal", "Team", "groupchat"],
            false,
            "2.1.1",
        )
    },
    app(
        WORKFLOWS,
        "Workflows",
        "Microsoft Corporation",
        "Automate tasks with flows",
        "#0b6a0b",
        &["Personal"],
        false,
        "3.0.0",
    ),
    App {
        device: &[],
        ..app(
            LISTS,
            "Lists",
            "Microsoft Corporation",
            "Track information in lists",
            "#0078d4",
            &["Personal", "Team"],
            false,
            "1.4.0",
        )
    },
    App {
        device: &["geolocation", "notifications"],
        ..app(
            SHIFTS,
            "Shifts",
            "Microsoft Corporation",
            "Manage schedules and shifts",
            "#005a9e",
            &["Personal"],
            false,
            "4.2.0",
        )
    },
    app(
        RECAPS,
        "Meeting Recaps",
        "Microsoft Corporation",
        "Recordings, transcripts and notes of your meetings",
        "#5b5fc7",
        &["Personal"],
        false,
        "1.0.0",
    ),
    app(
        "demo-tickets",
        "Contoso Tickets",
        "Contoso Ltd",
        "Raise a ticket from any message",
        "#a4262c",
        &["groupchat"],
        false,
        "1.0.0",
    ),
    app(
        POLLS,
        "Contoso Polls",
        "Contoso Ltd",
        "Quick polls for chats and teams",
        "#498205",
        &["groupchat", "Team"],
        false,
        "2.0.0",
    ),
    App {
        agent: true,
        paid: true,
        ..app(
            AGENT,
            "Fabrikam Agent",
            "Fabrikam Inc",
            "An agent that summarizes your week",
            "#5b5fc7",
            &["Personal"],
            false,
            "1.0.0",
        )
    },
    app(
        "demo-approvals",
        "Northwind Approvals",
        "Northwind Traders",
        "Approve requests without leaving Teams",
        "#ca5010",
        &["Personal", "groupchat"],
        false,
        "1.2.0",
    ),
];

#[allow(clippy::too_many_arguments)]
const fn app(
    id: &'static str,
    name: &'static str,
    developer: &'static str,
    blurb: &'static str,
    accent: &'static str,
    scopes: &'static [&'static str],
    core: bool,
    version: &'static str,
) -> App {
    App {
        id,
        name,
        developer,
        blurb,
        accent,
        scopes,
        core,
        agent: false,
        paid: false,
        version,
        device: &[],
    }
}

fn definition(a: &App) -> Value {
    let personal = a.scopes.contains(&"Personal");
    let mut caps = serde_json::Map::new();
    if personal && !a.agent {
        caps.insert("staticTabs".into(), json!({"scopes": ["Personal"]}));
    }
    if a.agent {
        caps.insert("bots".into(), json!({"scopes": ["Personal"]}));
        caps.insert("copilotExtensions".into(), json!({}));
    }
    if a.id == "demo-tickets" {
        caps.insert("messageExtensions".into(), json!({"scopes": ["groupchat"]}));
    } else if a.scopes.iter().any(|s| *s != "Personal") && !a.agent {
        let scopes: Vec<&str> = a
            .scopes
            .iter()
            .copied()
            .filter(|s| *s != "Personal")
            .collect();
        caps.insert("galleryTabs".into(), json!({ "scopes": scopes }));
    }
    let tabs = if personal && !a.agent {
        json!([
            {"entityId": "home", "name": a.name,
             "contentUrl": format!("https://apps.example.invalid/{}/{{locale}}", a.id),
             "scopes": ["Personal"]},
            {"entityId": "about", "scopes": ["Personal"]},
        ])
    } else {
        json!([])
    };
    json!({
        "id": a.id,
        "name": a.name,
        "version": a.version,
        "developerName": a.developer,
        "developerUrl": "https://example.invalid/",
        "shortDescription": a.blurb,
        "longDescription": format!("{} {} lets you work with it without leaving Teams.", a.blurb, a.name),
        "accentColor": a.accent,
        "smallImageUrl": "",
        "largeImageUrl": "",
        "screenshotUrls": [],
        "privacyUrl": "https://example.invalid/privacy",
        "termsOfUseUrl": "https://example.invalid/terms",
        "isCoreApp": a.core,
        "isMicrosoftOwned": a.developer.starts_with("Microsoft"),
        "categories": ["Productivity"],
        "staticTabs": tabs,
        "metaCapabilities": Value::Object(caps),
        "permissions": ["Identity"],
        "devicePermissions": a.device,
        "authorization": {"permissions": {"resourceSpecific": [{"name": "ChannelMessage.Read.Group", "type": "Application"}]}},
    })
}

/// Definitions of the demo apps the ids name.
pub fn definitions(ids: &[&str]) -> Value {
    Value::Array(
        APPS.iter()
            .filter(|a| ids.contains(&a.id))
            .map(definition)
            .collect(),
    )
}

/// The aggregated entitlements: the app bar in order, installed apps, and the definitions. The
/// installed Planner is one version behind the Store's.
pub fn installed() -> Value {
    let pinned = [
        ("demo-activity", true),
        ("demo-chat", true),
        ("demo-teams", true),
        ("demo-calendar", true),
        ("demo-calling", true),
        ("demo-onedrive", true),
        (PLANNER, false),
        (WORKFLOWS, false),
    ];
    let mut ents: Vec<Value> = pinned
        .iter()
        .enumerate()
        .map(|(i, (id, admin))| {
            let mut e = json!({
                "id": id,
                "state": if *admin { "InstalledAndPermanent" } else { "Installed" },
                "isAppBarPinned": true,
                "appBarOrder": i + 1,
            });
            e[if *admin {
                "isAdminPinned"
            } else {
                "isUserPinned"
            }] = json!(true);
            if *id == PLANNER {
                e["installedVersion"] = json!("1.8.0");
            }
            e
        })
        .collect();
    for id in [ONENOTE, LISTS, SHIFTS, RECAPS, "demo-tickets"] {
        ents.push(json!({"id": id, "state": "Installed"}));
    }
    ents.push(json!({"id": POLLS, "state": "Featured"}));
    let defs: serde_json::Map<String, Value> = APPS
        .iter()
        .filter(|a| ents.iter().any(|e| e["id"] == a.id))
        .map(|a| (a.id.to_owned(), definition(a)))
        .collect();
    json!({"value": {
        "userEntitlements": {"00000000-0000-0000-0000-000000000001": ents},
        "definitions": defs,
        "userEntitlementsHash": "0".repeat(64),
    }})
}

pub fn app_policy() -> Value {
    json!({"value": {"isSideloadingAllowed": false, "isUserPinningAllowed": true}})
}

fn entry(a: &App, rating: Option<(f64, u32)>, rank: u32) -> Value {
    let mut e = json!({"id": a.id, "version": a.version, "ranking": rank,
        "metaCapabilities": ["StaticTabs"]});
    if let Some((average, count)) = rating {
        e["rating"] = json!({"average": average, "count": count});
    }
    e
}

/// The Store home: every category with its ranked apps and a few ratings.
pub fn store_home() -> Value {
    let store_apps: Vec<&App> = APPS.iter().filter(|a| !a.core).collect();
    let rated = |a: &App| match a.id {
        PLANNER => Some((4.5, 119)),
        ONENOTE => Some((4.2, 87)),
        POLLS => Some((3.9, 12)),
        _ => None,
    };
    let list = |pick: &dyn Fn(&App) -> bool| -> Value {
        let apps: Vec<Value> = store_apps
            .iter()
            .filter(|a| pick(a))
            .enumerate()
            .map(|(i, a)| entry(a, rated(a), 1000 - i as u32))
            .collect();
        json!({"apps": apps, "totalAppsCount": apps.len(), "blockedAppsCount": 0})
    };
    json!({"appCategoriesDictionary": {
        "all": list(&|_| true),
        "popularApps": list(&|a| [PLANNER, ONENOTE, POLLS, AGENT].contains(&a.id)),
        "productivity": list(&|a| [PLANNER, ONENOTE, LISTS, SHIFTS].contains(&a.id)),
        "personalApps": list(&|a| a.scopes.contains(&"Personal")),
        "bots": list(&|a| a.agent || a.id == POLLS),
    }})
}

/// Search hits: every non-core app whose name or description has the query.
pub fn search(query: &str) -> Value {
    let q = query.to_lowercase();
    let results: Vec<Value> = APPS
        .iter()
        .filter(|a| !a.core)
        .filter(|a| a.name.to_lowercase().contains(&q) || a.blurb.to_lowercase().contains(&q))
        .map(|a| {
            let mut features = vec!["Available", if a.paid { "Paid" } else { "Free" }];
            if a.scopes.contains(&"Personal") {
                features.push("Personal");
            }
            if a.scopes.contains(&"Team") {
                features.push("NoContextTeamTab");
            }
            if a.scopes.contains(&"groupchat") {
                features.push("NoContextChatTab");
            }
            if a.agent {
                features.push("CopilotExtension");
                features.push("Certified");
            }
            json!({
                "appId": a.id, "searchScore": 1.0, "name": a.name, "developerName": a.developer,
                "version": a.version, "shortDescription": a.blurb, "accentColor": a.accent,
                "smallImageUrl": "", "appCategories": ["Productivity"], "features": features,
            })
        })
        .collect();
    json!({ "results": results })
}

/// "Your apps" in the Store: the installed apps.
pub fn your_apps() -> Value {
    let apps: serde_json::Map<String, Value> =
        [PLANNER, ONENOTE, LISTS, SHIFTS, "demo-tickets", WORKFLOWS]
            .iter()
            .filter_map(|id| APPS.iter().find(|a| a.id == *id))
            .map(|a| {
                (
                    a.id.to_owned(),
                    json!({"id": a.id, "version": a.version, "categories": ["Productivity"],
                       "metaCapabilities": ["StaticTabs"]}),
                )
            })
            .collect();
    json!({ "appIdToStoreEntitlementsDictionary": apps })
}

/// Graph `teamsApp` items for "Built for your org".
pub fn org_apps() -> Vec<Value> {
    vec![json!({
        "id": "demo-org-app", "externalId": "demo-org-ext", "displayName": "Contoso Intranet",
        "distributionMethod": "organization",
        "appDefinitions": [{"version": "1.0.0", "shortDescription": "News from your company",
                            "description": "News from your company", "bot": null}],
    })]
}

// -- Planner and To Do --------------------------------------------------------------------

pub fn todo_lists() -> Value {
    json!({"value": [
        {"id": "todo-1", "displayName": "Tasks", "isShared": false, "wellknownListName": "defaultList"},
        {"id": "todo-2", "displayName": "Groceries", "isShared": false, "wellknownListName": "none"},
        {"id": "todo-3", "displayName": "Flagged email", "isShared": false, "wellknownListName": "flaggedEmails"},
    ]})
}

pub fn todo_tasks(list: &str, now_ms: i64) -> Value {
    let due = |d: i64| json!({"dateTime": format!("{}.0000000", iso(now_ms + d * MS_DAY).trim_end_matches('Z')), "timeZone": "UTC"});
    match list {
        "todo-1" => json!({"value": [
            {"id": "t1", "title": "Send the quarterly report", "status": "notStarted", "importance": "high", "dueDateTime": due(2)},
            {"id": "t2", "title": "Book the team offsite", "status": "inProgress", "importance": "normal"},
            {"id": "t3", "title": "Review the design doc", "status": "completed", "importance": "normal",
             "completedDateTime": due(-1)},
        ]}),
        "todo-2" => json!({"value": [
            {"id": "g1", "title": "Coffee beans", "status": "notStarted", "importance": "low"},
        ]}),
        _ => json!({"value": []}),
    }
}

pub fn planner_plans() -> Value {
    json!({"value": [
        {"id": "plan-1", "title": "Sprint board", "owner": "00000000-0000-0000-0000-0000000000a1",
         "createdDateTime": "2026-10-01T09:00:00Z"},
        {"id": "plan-2", "title": "Launch plan", "owner": "00000000-0000-0000-0000-0000000000a2",
         "createdDateTime": "2026-10-05T09:00:00Z"},
    ]})
}

pub fn planner_buckets(plan: &str) -> Value {
    match plan {
        "plan-1" => json!({"value": [
            {"id": "b1", "name": "To do", "planId": "plan-1"},
            {"id": "b2", "name": "Doing", "planId": "plan-1"},
            {"id": "b3", "name": "Done", "planId": "plan-1"},
        ]}),
        _ => json!({"value": [{"id": "b4", "name": "Backlog", "planId": "plan-2"}]}),
    }
}

pub fn planner_tasks(plan: Option<&str>, now_ms: i64) -> Value {
    let all = vec![
        (
            "p1",
            "plan-1",
            "b1",
            "Draft the release notes",
            0,
            3,
            Some(3),
        ),
        ("p2", "plan-1", "b2", "Fix the sync bug", 50, 1, Some(1)),
        ("p3", "plan-1", "b3", "Update the docs", 100, 5, None),
        ("p4", "plan-2", "b4", "Pick the launch date", 0, 5, Some(14)),
    ];
    let tasks: Vec<Value> = all
        .into_iter()
        .filter(|t| plan.is_none_or(|p| p == t.1))
        .map(|(id, plan, bucket, title, pct, prio, due)| {
            let mut t = json!({"id": id, "planId": plan, "bucketId": bucket, "title": title,
                "@odata.etag": format!("W/\"etag-{id}\""),
                "percentComplete": pct, "priority": prio, "assignments": {"someone": {}}});
            if let Some(d) = due {
                t["dueDateTime"] = json!(iso(now_ms + d * MS_DAY));
            }
            t
        })
        .collect();
    json!({ "value": tasks })
}

// -- Workflows -----------------------------------------------------------------------------

pub fn flow_environments() -> Value {
    json!({"value": [{"name": "Default-demo", "properties": {"displayName": "Demo (default)", "isDefault": true}}]})
}

pub fn flows(now_ms: i64) -> Value {
    json!({"value": [
        {"name": "flow-1", "properties": {"displayName": "Post a message when a form is submitted", "state": "Started",
            "createdTime": iso(now_ms - 30 * MS_DAY), "lastModifiedTime": iso(now_ms - 2 * MS_DAY),
            "definitionSummary": {"triggers": [{"type": "OpenApiConnectionWebhook"}], "actions": [{}, {}, {}]}}},
        {"name": "flow-2", "properties": {"displayName": "Remind me every Monday", "state": "Started",
            "createdTime": iso(now_ms - 60 * MS_DAY), "lastModifiedTime": iso(now_ms - 9 * MS_DAY),
            "definitionSummary": {"triggers": [{"type": "Recurrence"}], "actions": [{}]}}},
        {"name": "flow-3", "properties": {"displayName": "Copy a file to OneDrive", "state": "Stopped",
            "createdTime": iso(now_ms - 90 * MS_DAY), "lastModifiedTime": iso(now_ms - 40 * MS_DAY),
            "definitionSummary": {"triggers": [{"kind": "Button"}], "actions": [{}, {}]}}},
        {"name": "flow-4", "properties": {"displayName": "Send my weekly status", "state": "Started",
            "createdTime": iso(now_ms - 20 * MS_DAY), "lastModifiedTime": iso(now_ms - 5 * MS_DAY),
            "definitionSummary": {"triggers": [{"kind": "Button"}], "actions": [{}]}}},
    ]})
}

pub fn flow_runs(flow: &str, now_ms: i64) -> Value {
    if flow == "flow-3" {
        return json!({"value": []});
    }
    let run = |name: &str, status: &str, ago_min: i64, secs: i64| {
        let start = now_ms - ago_min * 60_000;
        json!({"name": name, "properties": {"status": status, "startTime": iso(start),
            "endTime": iso(start + secs * 1000), "trigger": {"name": "manual"}}})
    };
    json!({"value": [
        run("run-1", "Succeeded", 20, 4),
        run("run-2", "Failed", 600, 2),
        run("run-3", "Succeeded", 1500, 5),
    ]})
}

// -- OneNote -------------------------------------------------------------------------------

pub fn notebooks(now_ms: i64) -> Value {
    json!({"value": [{
        "id": "nb-1", "displayName": "Team notebook", "lastModifiedDateTime": iso(now_ms - 3 * MS_DAY),
        "links": {"oneNoteWebUrl": {"href": "https://example.invalid/onenote/nb-1"}},
        "sectionsUrl": "https://graph.microsoft.com/v1.0/demo/nb-1/sections",
        "sectionGroupsUrl": "https://graph.microsoft.com/v1.0/demo/nb-1/sectionGroups",
    }]})
}

pub fn sections(now_ms: i64) -> Value {
    json!({"value": [
        {"id": "sec-1", "displayName": "Meeting notes", "lastModifiedDateTime": iso(now_ms - MS_DAY),
         "pagesUrl": "https://graph.microsoft.com/v1.0/demo/sec-1/pages"},
        {"id": "sec-2", "displayName": "Ideas", "lastModifiedDateTime": iso(now_ms - 5 * MS_DAY),
         "pagesUrl": "https://graph.microsoft.com/v1.0/demo/sec-2/pages"},
    ]})
}

pub fn pages(section_url: &str, now_ms: i64) -> Value {
    if section_url.contains("sec-2") {
        return json!({"value": []});
    }
    json!({"value": [
        {"id": "pg-1", "title": "Weekly sync", "lastModifiedDateTime": iso(now_ms - MS_DAY),
         "links": {"oneNoteWebUrl": {"href": "https://example.invalid/onenote/pg-1"}},
         "contentUrl": "https://graph.microsoft.com/v1.0/demo/pg-1/content"},
        {"id": "pg-2", "title": "Retro", "lastModifiedDateTime": iso(now_ms - 4 * MS_DAY),
         "contentUrl": "https://graph.microsoft.com/v1.0/demo/pg-2/content"},
    ]})
}

pub fn page_html(content_url: &str) -> String {
    if content_url.contains("pg-1") {
        "<html><head><title>Weekly sync</title></head><body><p>Agenda</p><p>Ship the Store view &amp; Planner</p></body></html>".into()
    } else {
        "<html><head><title>Retro</title></head><body><p>What went well</p></body></html>".into()
    }
}

// -- Lists ---------------------------------------------------------------------------------

pub fn sites() -> Value {
    json!({"value": [{"id": "site-1", "displayName": "Project Falcon", "webUrl": "https://example.invalid/sites/falcon"}]})
}

pub fn site_lists(now_ms: i64) -> Value {
    json!({"value": [
        {"id": "list-1", "displayName": "Issue tracker", "webUrl": "https://example.invalid/lists/1",
         "lastModifiedDateTime": iso(now_ms - MS_DAY), "list": {"template": "genericList", "hidden": false}},
        {"id": "list-2", "displayName": "Documents", "webUrl": "https://example.invalid/lists/2",
         "list": {"template": "documentLibrary", "hidden": false}},
    ]})
}

pub fn list_columns() -> Value {
    json!({"value": [
        {"name": "Title", "displayName": "Title", "text": {}},
        {"name": "Status", "displayName": "Status", "choice": {}},
        {"name": "Owner", "displayName": "Owner", "text": {}},
        {"name": "Done", "displayName": "Done", "boolean": {}},
        {"name": "_hidden", "displayName": "Hidden", "text": {}, "hidden": true},
    ]})
}

pub fn list_items() -> Value {
    json!({"value": [
        {"id": "1", "fields": {"Title": "Login fails on mobile", "Status": "Open", "Owner": "Ada", "Done": false}},
        {"id": "2", "fields": {"Title": "Slow start-up", "Status": "In progress", "Owner": "Grace", "Done": false}},
        {"id": "3", "fields": {"Title": "Typo on the pricing page", "Status": "Closed", "Owner": "Ada", "Done": true}},
    ]})
}

// -- Meeting Recaps ------------------------------------------------------------------------

/// The meeting content service's list: a recorded and transcribed meeting with an AI summary, a
/// meeting with only a recording, and one with only a transcript. (Only the tests read it: the
/// service is an optional source the demo does not need.)
#[cfg(test)]
pub fn recap_meetings(now_ms: i64) -> Value {
    let rec = |item: &str| {
        json!({"type": "Recording", "metadata": {"driveId": "b!demo", "driveItemId": item,
            "webUrl": format!("https://contoso-my.sharepoint.com.invalid/{item}.mp4")}})
    };
    let tr = |item: &str| {
        json!({"type": "TranscriptV2", "metadata": {"transcriptId": "tr-1", "driveId": "b!demo",
            "driveItemId": item}})
    };
    json!({"value": [
        {"threadInfo": {"threadId": "19:meeting_demo1@thread.v2", "iCalUid": "uid-demo-1",
            "subject": "Weekly sync", "start": iso(now_ms - 2 * MS_DAY),
            "end": iso(now_ms - 2 * MS_DAY + 1_800_000),
            "organizer": {"emailAddress": "ada@example.invalid"}},
         "resources": [rec("01DEMO1"), tr("01DEMO1"), {"type": "AISummary", "metadata": {}}]},
        {"threadInfo": {"threadId": "19:meeting_demo2@thread.v2", "iCalUid": "uid-demo-2",
            "subject": "Design review", "start": iso(now_ms - 5 * MS_DAY),
            "end": iso(now_ms - 5 * MS_DAY + 3_600_000)},
         "resources": [rec("01DEMO2")]},
        {"threadInfo": {"threadId": "19:meeting_demo3@thread.v2", "iCalUid": "uid-demo-3",
            "subject": "Retro", "start": iso(now_ms - 9 * MS_DAY)},
         "resources": [tr("01DEMO3")]},
    ]})
}

/// A WebVTT transcript with voice tags, as Teams writes it.
pub fn transcript_vtt() -> &'static str {
    "WEBVTT\n\n00:00:02.000 --> 00:00:05.000\n<v Ada>Welcome everyone.</v>\n\n\
     00:00:06.000 --> 00:00:09.500\n<v Sam>Let us start with the release.</v>\n"
}

/// A meeting chat's tabs (Graph `chats/{id}/tabs?$expand=teamsApp`).
pub fn meeting_tabs() -> Value {
    json!({"value": [
        {"displayName": "Whiteboard", "webUrl": "https://example.invalid/whiteboard",
         "teamsApp": {"id": "wb", "displayName": "Whiteboard"}},
        {"displayName": "Notes", "webUrl": "https://example.invalid/notes",
         "teamsApp": {"id": "notes", "displayName": "Meeting notes"}},
    ]})
}

/// The calendar the Recaps home is built from (Graph `calendarView` events): online meetings of
/// the last month, one of them canceled, plus what must stay out of the list (an in-person
/// event, one older than 30 days, one still to come).
pub fn recap_events(now_ms: i64) -> Value {
    let at = |ms: i64| json!({"dateTime": iso(ms), "timeZone": "UTC"});
    let ev = |id: &str, subject: &str, start: i64, mins: i64, thread: Option<&str>, who: &str| {
        let mut v = json!({"id": id, "subject": subject, "start": at(start),
            "end": at(start + mins * 60_000), "type": "singleInstance",
            "organizer": {"emailAddress": {"name": who, "address": "organizer@example.invalid"}}});
        if let Some(t) = thread {
            v["isOnlineMeeting"] = json!(true);
            v["onlineMeeting"] = json!({"joinUrl": format!(
                "https://teams.microsoft.com/l/meetup-join/19%3ameeting_{t}%40thread.v2/0"
            )});
        }
        v
    };
    let mut canceled = ev(
        "ev-canceled",
        "Budget review",
        now_ms - 4 * MS_DAY,
        30,
        Some("demo4"),
        "Grace Example",
    );
    canceled["isCancelled"] = json!(true);
    json!({"value": [
        ev("ev-sync", "Weekly sync", now_ms - 2 * MS_DAY, 30, Some("demo1"), "Ada Example"),
        canceled,
        ev("ev-lunch", "Team lunch", now_ms - 3 * MS_DAY, 60, None, "Ada Example"),
        ev("ev-design", "Design review", now_ms - 9 * MS_DAY, 60, Some("demo2"), "Sam Example"),
        ev("ev-retro", "Retro", now_ms - 16 * MS_DAY, 45, Some("demo3"), "Ada Example"),
        ev("ev-old", "Old planning", now_ms - 40 * MS_DAY, 60, Some("demo5"), "Ada Example"),
        ev("ev-next", "Next sync", now_ms + MS_DAY, 30, Some("demo6"), "Ada Example"),
    ]})
}

/// A finished recording message of a meeting chat, as the chat service sends it.
fn recording_message(id: i64, item: &str, length: &str, with_transcript: bool) -> Value {
    let types = if with_transcript {
        "Recording+Transcript"
    } else {
        "Recording"
    };
    json!({"id": id.to_string(), "messagetype": "RichText/Media_CallRecording",
    "composetime": iso(id), "from": "8:orgid:demo-organizer",
    "content": format!(
        "<URIObject type=\"Video.2/CallRecording.1\"><RecordingStatus status=\"Success\" \
         code=\"200\"></RecordingStatus><OriginalName v=\"{item}.mp4\" />\
         <RecordingContent contentTypes=\"{types}\" duration=\"{length}\">\
         <item type=\"onedriveForBusinessVideo\" \
         uri=\"https://contoso-my.sharepoint.com.invalid/{item}.mp4\" driveId=\"b!demo\" \
         driveItemId=\"{item}\" /></RecordingContent></URIObject>"
    )})
}

/// The messages of a meeting chat (chat service history): the weekly sync was recorded in two
/// parts, the second with a transcript, and an agenda was shared; the design review has one
/// recording; the others have nothing.
pub fn meeting_chat(thread: &str, now_ms: i64) -> Value {
    let messages = if thread.contains("demo1") {
        let start = now_ms - 2 * MS_DAY;
        vec![
            json!({"id": (start - 60_000).to_string(), "messagetype": "RichText/Html",
                "composetime": iso(start - 60_000), "imdisplayname": "Ada Example",
                "content": "<p>Agenda attached</p>",
                "properties": {"files": "[{\"fileName\":\"Agenda.docx\",\"fileType\":\"docx\",\
                    \"objectUrl\":\"https://contoso.sharepoint.com.invalid/Agenda.docx\"}]"}}),
            json!({"id": (start + 60_000).to_string(),
                "messagetype": "RichText/Media_CallRecording", "composetime": iso(start + 60_000),
                "content": "<URIObject type=\"Video.2/CallRecording.1\"><RecordingStatus \
                    status=\"Started\" code=\"200\"></RecordingStatus></URIObject>"}),
            recording_message(start + 20 * 60_000, "01DEMO1", "0:18:00.00", false),
            recording_message(start + 31 * 60_000, "01DEMO1B", "0:10:05.00", true),
            json!({"id": (start + 32 * 60_000).to_string(),
                "messagetype": "RichText/Media_CallTranscript",
                "composetime": iso(start + 32 * 60_000),
                "content": "{\\\"callId\\\":\\\"call-demo\\\",\\\"isExportedToOdsp\\\":true}"}),
        ]
    } else if thread.contains("demo2") {
        vec![recording_message(
            now_ms - 9 * MS_DAY + 61 * 60_000,
            "01DEMO2",
            "0:58:00.00",
            false,
        )]
    } else {
        vec![]
    };
    json!({"messages": messages})
}
