//! `kelma-cli store …`: the app bar, the flyout and the Store (crate `apps`) against the live
//! service. Reads only: install, uninstall, update, pin and move print the request they would
//! send (dry run) and never send it.

use anyhow::{Context, Result, bail};
use apps::{AddTarget, AppDetails, Filter, PinChange, Scope, Target, WorksIn};
use reqwest::Method;
use serde_json::Value;
use teams_proto::Client;
use teams_proto::broker::Audience;

/// Prints the shape of a JSON value: keys, array lengths and string lengths. String values are
/// printed only for the keys named in `KELMA_SHOW` (comma separated).
fn shape(v: &Value, indent: usize, key: &str) {
    let pad = " ".repeat(indent);
    match v {
        Value::Object(m) => {
            println!("{pad}{key}: object({})", m.len());
            for (k, x) in m {
                shape(x, indent + 2, k);
            }
        }
        Value::Array(a) => {
            println!("{pad}{key}: array({})", a.len());
            if let Some(f) = a.first() {
                shape(f, indent + 2, "[0]");
            }
        }
        Value::String(s)
            if std::env::var("KELMA_SHOW").is_ok_and(|w| w.split(',').any(|k| k == key)) =>
        {
            println!("{pad}{key}: {s:?}")
        }
        Value::String(s) => println!("{pad}{key}: string({})", s.len()),
        Value::Number(n) => println!("{pad}{key}: {n}"),
        Value::Bool(b) => println!("{pad}{key}: {b}"),
        Value::Null => println!("{pad}{key}: null"),
    }
}

/// `store probe <mt|teams|graph> <path or url>...`: GET (never anything else) and print shapes.
async fn probe(client: &Client, which: &str, paths: &[String]) -> Result<()> {
    for path in paths {
        println!("== {path}");
        let url = match which {
            "mt" => format!("{}{path}", client.middle_tier().await),
            "teams" if path.starts_with("https://") => path.clone(),
            "teams" => format!("https://teams.microsoft.com{path}"),
            "graph" if path.starts_with("https://") => path.clone(),
            "graph" => format!("https://graph.microsoft.com{path}"),
            other => bail!("unknown backend {other}"),
        };
        let (status, text) = if which == "graph" {
            client
                .request_as(&Audience::Graph, Method::GET, &url, None, &[])
                .await?
        } else {
            client
                .request(Method::GET, &url, None, &[("x-ms-client-caller", "kelma")])
                .await?
        };
        println!("status {status}");
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => shape(&v, 0, "$"),
            Err(_) => println!("body: {} bytes, not JSON", text.len()),
        }
    }
    Ok(())
}

fn works(w: WorksIn) -> String {
    let mut s: Vec<&str> = Vec::new();
    for (on, name) in [
        (w.personal, "personal"),
        (w.team, "team"),
        (w.chat, "chat"),
        (w.meeting, "meeting"),
    ] {
        if on {
            s.push(name);
        }
    }
    if s.is_empty() {
        "-".into()
    } else {
        s.join(",")
    }
}

/// The value after `flag`, if present.
fn flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// `--team <thread>`, `--chat <thread>` or `--meeting <thread>`, else personal scope.
fn target(args: &[String]) -> Target {
    if let Some(t) = flag(args, "--team") {
        Target::Team(t.into())
    } else if let Some(c) = flag(args, "--chat") {
        Target::Chat(c.into())
    } else if let Some(m) = flag(args, "--meeting") {
        Target::Meeting(m.into())
    } else {
        Target::Me
    }
}

fn filter(args: &[String]) -> Filter {
    let has = |f: &str| args.iter().any(|a| a == f);
    Filter {
        scope: [
            ("--personal", Scope::Personal),
            ("--team", Scope::Team),
            ("--chat", Scope::Chat),
            ("--meeting", Scope::Meeting),
        ]
        .into_iter()
        .find(|(f, _)| has(f))
        .map(|(_, s)| s),
        agents: if has("--agents") {
            Some(true)
        } else if has("--apps") {
            Some(false)
        } else {
            None
        },
    }
}

async fn app_page(client: &Client, id: &str) -> Result<()> {
    let def = apps::definitions(client, &[id])
        .await?
        .into_iter()
        .next()
        .context("no such app")?;
    let home = apps::store_home(client).await?;
    let entry = home
        .categories
        .iter()
        .flat_map(|c| &c.apps)
        .find(|e| e.id == def.id);
    let hits = apps::search(client, &def.name).await?;
    let hit = hits.iter().find(|h| h.id == def.id);
    let d = AppDetails::new(def, entry, hit);
    let def = &d.definition;
    println!("{} {} by {}", def.name, def.version, def.developer);
    println!("works in: {}", works(def.works_in));
    let add: Vec<&str> = d
        .add_targets()
        .iter()
        .map(|t| match t {
            AddTarget::Me => "for me",
            AddTarget::Team => "to a team",
            AddTarget::Chat => "to a chat",
            AddTarget::Meeting => "to a meeting",
        })
        .collect();
    println!("add: {}", add.join(", "));
    match d.rating {
        Some(r) => println!("rating: {:.1} from {} ratings", r.average, r.count),
        None => println!("rating: none"),
    }
    println!("price: {:?}, certification: {:?}", d.price, d.certification);
    println!(
        "{} screenshots, video: {}, privacy: {}, terms: {}",
        def.screenshots.len(),
        def.video_url.is_some(),
        def.privacy_url.is_some(),
        def.terms_url.is_some()
    );
    println!(
        "permissions: {:?}, RSC: {}, device: {:?}",
        def.permissions,
        def.rsc_permissions.len(),
        def.device_permissions
    );
    println!(
        "personal tabs: {}, categories: {:?}",
        def.personal_tabs().count(),
        def.categories
    );
    println!("share: {}", def.share_link());
    Ok(())
}

pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("bar") => {
            let installed = apps::installed_apps(client).await?;
            let policy = apps::app_policy(client).await?;
            println!(
                "{} entitled apps, user pinning allowed: {}",
                installed.entitlements.len(),
                policy.user_pinning_allowed
            );
            for (e, d) in installed.app_bar() {
                let by = match (e.admin_pinned, e.user_pinned) {
                    (true, _) => "admin",
                    (_, true) => "user",
                    _ => "-",
                };
                println!("{:>3} {:<24} pinned by {by}", e.order.unwrap_or(0), d.name);
            }
        }
        Some("more") => {
            let installed = apps::installed_apps(client).await?;
            let more = installed.more_apps(arg(1).unwrap_or_default());
            println!("{} more apps", more.len());
            for d in more {
                println!(
                    "  {:<28} {} personal tab(s)",
                    d.name,
                    d.personal_tabs().count()
                );
            }
        }
        Some("home") => {
            let home = apps::store_home(client).await?;
            println!("{} categories", home.categories.len());
            for c in &home.categories {
                println!(
                    "  {:<32} {:>5} apps, {:>3} blocked, {:>3} listed, {:>3} rated",
                    c.key,
                    c.total,
                    c.blocked,
                    c.apps.len(),
                    c.apps.iter().filter(|a| a.rating.is_some()).count()
                );
            }
        }
        Some("search") => {
            let q = arg(1).context("usage: store search <query> [--personal|--team|--chat|--meeting] [--agents|--apps]")?;
            let f = filter(&args[2..]);
            let hits: Vec<_> = apps::search(client, q)
                .await?
                .into_iter()
                .filter(|h| f.matches(h.works_in, h.is_agent))
                .collect();
            println!("{} hits", hits.len());
            for h in hits.iter().take(20) {
                println!(
                    "  {:<32} {:<26} {:?} {}",
                    h.name,
                    works(h.works_in),
                    h.price,
                    if h.is_agent { "agent" } else { "app" }
                );
            }
        }
        Some("app") => app_page(client, arg(1).context("usage: store app <app id>")?).await?,
        Some("mine") => {
            let mine = apps::your_apps(client).await?;
            let installed = apps::installed_apps(client).await?;
            let updates = installed
                .entitlements
                .iter()
                .filter(|e| e.has_update(installed.definition(&e.id)))
                .count();
            println!(
                "{} apps added from the Store, {updates} updates available",
                mine.len()
            );
        }
        Some("org") => {
            let org = apps::org_apps(client).await?;
            println!(
                "{} apps built for your org, {} with a bot",
                org.len(),
                org.iter().filter(|a| a.has_bot).count()
            );
        }
        Some("settings") => {
            let policy = apps::app_policy(client).await?;
            println!(
                "custom app upload allowed: {}, user pinning allowed: {}",
                policy.sideloading_allowed, policy.user_pinning_allowed
            );
            let installed = apps::installed_apps(client).await?;
            for p in apps::DevicePermission::ALL {
                let n = installed
                    .definitions
                    .values()
                    .filter(|d| d.device_permissions.contains(&p))
                    .count();
                println!("  {:<14} asked by {n} apps", p.as_str());
            }
        }
        Some("install") => {
            let id = arg(1)
                .context("usage: store install <app id> [--team|--chat|--meeting <thread>]")?;
            let def = apps::definitions(client, &[id])
                .await?
                .into_iter()
                .next()
                .context("no such app")?;
            let base = apps::api::base(client).await;
            println!(
                "{}",
                apps::install_request(&base, &target(args), &def.raw).dry_run()
            );
        }
        Some("uninstall") => {
            let id = arg(1)
                .context("usage: store uninstall <app id> [--team|--chat|--meeting <thread>]")?;
            let base = apps::api::base(client).await;
            println!(
                "{}",
                apps::uninstall_request(&base, &target(args), id).dry_run()
            );
        }
        Some("update") => {
            let id = arg(1).context("usage: store update <app id>")?;
            let def = apps::definitions(client, &[id])
                .await?
                .into_iter()
                .next()
                .context("no such app")?;
            let base = apps::api::base(client).await;
            println!(
                "{}",
                apps::update_request(&base, &target(args), id, &def.version).dry_run()
            );
        }
        Some(cmd @ ("pin" | "unpin" | "move")) => {
            let id =
                arg(1).context("usage: store pin|unpin <app id> | store move <app id> <index>")?;
            let change = match cmd {
                "pin" => PinChange::Pin(id.into()),
                "unpin" => PinChange::Unpin(id.into()),
                _ => PinChange::Move(id.into(), arg(2).context("missing index")?.parse()?),
            };
            let installed = apps::installed_apps(client).await?;
            let base = apps::api::base(client).await;
            println!(
                "{}",
                apps::pin_request(&base, &installed, &change).dry_run()
            );
        }
        Some("probe") => {
            let which = arg(1).context("usage: store probe <mt|teams|graph> <path>...")?;
            probe(client, which, &args[2..]).await?
        }
        _ => bail!(
            "usage: store <bar|more [q]|home|search q|app id|mine|org|settings|install id|uninstall id|update id|pin id|unpin id|move id n|probe>"
        ),
    }
    Ok(())
}
