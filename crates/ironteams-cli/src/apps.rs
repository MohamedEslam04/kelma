//! Read-only commands for the built-in apps: `tasks`, `flows`, `onenote` and `lists`.

use anyhow::{Context, Result, bail};
use teams_proto::Client;
use teams_proto::lists::field_text;
use teams_proto::onenote::{Notebook, page_text};

fn day(ms: Option<i64>) -> String {
    ms.map_or("-".into(), |ms| {
        let days = ms.div_euclid(86_400_000);
        // Civil date from days since 1970-01-01 (Howard Hinnant).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!("{y:04}-{m:02}-{d:02}")
    })
}

/// The `n`th (1-based) argument as an index into a list of `len` items.
fn pick(args: &[String], n: usize, len: usize, what: &str) -> Result<Option<usize>> {
    let Some(a) = args.get(n) else {
        return Ok(None);
    };
    let i: usize = a
        .parse()
        .with_context(|| format!("{what}: expected a number"))?;
    if i == 0 || i > len {
        bail!("{what}: {i} is out of range 1..={len}");
    }
    Ok(Some(i - 1))
}

/// `ironteams-cli tasks [todo <n> | plan <n>]`
pub async fn tasks(client: &Client, args: &[String]) -> Result<()> {
    let lists = client.todo_lists().await?;
    let mut plans = client.my_planner_plans().await?;
    for g in client.joined_group_ids().await? {
        for p in client.group_planner_plans(&g).await.unwrap_or_default() {
            if !plans.iter().any(|o| o.id == p.id) {
                plans.push(p);
            }
        }
    }
    match args.first().map(String::as_str) {
        Some("todo") => {
            let i = pick(args, 1, lists.len(), "todo list")?.context("usage: tasks todo <n>")?;
            for t in client.todo_tasks(&lists[i].id).await? {
                println!(
                    "{} {:<8} due {}  {}",
                    if t.is_completed() { "[x]" } else { "[ ]" },
                    t.importance,
                    day(t.due_ms),
                    t.title
                );
            }
        }
        Some("plan") => {
            let i = pick(args, 1, plans.len(), "plan")?.context("usage: tasks plan <n>")?;
            let buckets = client.plan_buckets(&plans[i].id).await?;
            let tasks = client.plan_tasks(&plans[i].id).await?;
            for b in &buckets {
                println!("== {}", b.name);
                for t in tasks.iter().filter(|t| t.bucket_id == b.id) {
                    println!(
                        "  {:>3}% p{} due {}  {}",
                        t.percent_complete,
                        t.priority,
                        day(t.due_ms),
                        t.title
                    );
                }
            }
        }
        _ => {
            println!("To Do lists:");
            for (i, l) in lists.iter().enumerate() {
                let tasks = client.todo_tasks(&l.id).await?;
                let open = tasks.iter().filter(|t| !t.is_completed()).count();
                println!(
                    "  {}. {}  ({open} open, {} done)",
                    i + 1,
                    l.name,
                    tasks.len() - open
                );
            }
            let mine = client.my_planner_tasks().await?;
            println!("Planner tasks assigned or created by you: {}", mine.len());
            println!("Plans:");
            for (i, p) in plans.iter().enumerate() {
                println!("  {}. {}", i + 1, p.title);
            }
        }
    }
    Ok(())
}

/// `ironteams-cli flows [runs <n> [top]]`
pub async fn flows(client: &Client, args: &[String]) -> Result<()> {
    let envs = client.flow_environments().await?;
    let env = envs
        .iter()
        .find(|e| e.is_default)
        .or(envs.first())
        .context("no Power Automate environment")?;
    let flows = client.flows(&env.name).await?;
    match args.first().map(String::as_str) {
        Some("runs") => {
            let i = pick(args, 1, flows.len(), "flow")?.context("usage: flows runs <n> [top]")?;
            let top = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(10);
            for r in client.flow_runs(&env.name, &flows[i].id, top).await? {
                let secs = match (r.start_ms, r.end_ms) {
                    (Some(s), Some(e)) => format!("{}s", (e - s) / 1000),
                    _ => "-".into(),
                };
                println!(
                    "{}  {:<10} {:>5}  {}",
                    day(r.start_ms),
                    r.status,
                    secs,
                    r.trigger.unwrap_or_default()
                );
            }
        }
        _ => {
            for (i, f) in flows.iter().enumerate() {
                println!(
                    "{}. {} [{}] trigger {} ({} actions) modified {}",
                    i + 1,
                    f.display_name,
                    f.state,
                    f.trigger.as_deref().unwrap_or("-"),
                    f.action_count,
                    day(f.modified_ms)
                );
            }
            println!("({} flows)", flows.len());
        }
    }
    Ok(())
}

async fn all_notebooks(client: &Client) -> Result<Vec<Notebook>> {
    let mut out = client.my_notebooks().await?;
    for g in client.joined_group_ids().await? {
        for n in client.group_notebooks(&g).await.unwrap_or_default() {
            if !out.iter().any(|o| o.id == n.id) {
                out.push(n);
            }
        }
    }
    Ok(out)
}

/// `ironteams-cli onenote [<notebook> [<section> [<page>]]]`, numbers from the previous listing.
pub async fn onenote(client: &Client, args: &[String]) -> Result<()> {
    let notebooks = all_notebooks(client).await?;
    let Some(n) = pick(args, 0, notebooks.len(), "notebook")? else {
        for (i, n) in notebooks.iter().enumerate() {
            println!("{}. {}  modified {}", i + 1, n.name, day(n.modified_ms));
        }
        println!("({} notebooks)", notebooks.len());
        return Ok(());
    };
    let url = notebooks[n]
        .sections_url
        .as_deref()
        .context("notebook has no sections link")?;
    let sections = client
        .notebook_sections(Some(url), notebooks[n].section_groups_url.as_deref())
        .await?;
    let Some(s) = pick(args, 1, sections.len(), "section")? else {
        for (i, s) in sections.iter().enumerate() {
            let group = if s.group_path.is_empty() {
                String::new()
            } else {
                format!("{} / ", s.group_path)
            };
            println!(
                "{}. {group}{}  modified {}",
                i + 1,
                s.name,
                day(s.modified_ms)
            );
        }
        println!("({} sections)", sections.len());
        return Ok(());
    };
    let url = sections[s]
        .pages_url
        .as_deref()
        .context("section has no pages link")?;
    let pages = client.section_pages(url).await?;
    let Some(p) = pick(args, 2, pages.len(), "page")? else {
        for (i, p) in pages.iter().enumerate() {
            println!("{}. {}  modified {}", i + 1, p.title, day(p.modified_ms));
        }
        println!("({} pages)", pages.len());
        return Ok(());
    };
    let url = pages[p]
        .content_url
        .as_deref()
        .context("page has no content link")?;
    println!("{}", page_text(&client.page_html(url).await?));
    Ok(())
}

/// `ironteams-cli lists [<site> [<list>]]`, numbers from the previous listing.
pub async fn lists(client: &Client, args: &[String]) -> Result<()> {
    let mut sites = client.team_sites().await?;
    // The user's own OneDrive site is reachable too, but it holds no lists.
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    let Some(s) = pick(args, 0, sites.len(), "site")? else {
        for (i, s) in sites.iter().enumerate() {
            match client.site_lists(&s.id).await {
                Ok(lists) => {
                    let n = lists.iter().filter(|l| l.is_user_list()).count();
                    println!(
                        "{}. {}  ({n} lists, {} libraries and system lists)",
                        i + 1,
                        s.name,
                        lists.len() - n
                    );
                }
                Err(e) => println!("{}. {}  (error: {e})", i + 1, s.name),
            }
        }
        return Ok(());
    };
    let lists = client.site_lists(&sites[s].id).await?;
    let Some(l) = pick(args, 1, lists.len(), "list")? else {
        for (i, l) in lists.iter().enumerate() {
            println!(
                "{}. {}  [{}{}] modified {}",
                i + 1,
                l.name,
                l.template.as_deref().unwrap_or("-"),
                if l.hidden { ", hidden" } else { "" },
                day(l.modified_ms)
            );
        }
        return Ok(());
    };
    let columns: Vec<_> = client
        .list_columns(&sites[s].id, &lists[l].id)
        .await?
        .into_iter()
        .filter(|c| !c.hidden)
        .collect();
    println!(
        "columns: {}",
        columns
            .iter()
            .map(|c| format!("{} ({})", c.display_name, c.kind))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let items = client.list_items(&sites[s].id, &lists[l].id, 1).await?;
    for it in &items {
        let row: Vec<String> = columns
            .iter()
            .filter_map(|c| it.fields.get(&c.name))
            .map(field_text)
            .collect();
        println!("{}", row.join(" | "));
    }
    println!("({} items)", items.len());
    Ok(())
}
