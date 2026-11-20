//! `kelma-cli team …`: teams and channels management.
//!
//! Every write goes to the sandbox team by default. Its ids live in the file named by
//! `KELMA_SANDBOX_FILE` (outside the repo), so a test run never touches a real team.

use anyhow::{Context, Result, bail};
use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::Client;
use teams_proto::tags::Tag;
use teams_proto::teams::{self as t, Access, ChannelKind, Moderation, Role, TeamRef};

/// Prints the key structure of a JSON value: types and lengths, plus the value of a few
/// enum-like keys. Never prints names, ids or content.
fn shape(v: &Value, indent: usize, key: &str) {
    const SAFE: [&str; 26] = [
        "channelType",
        "membershipType",
        "role",
        "visibility",
        "type",
        "status",
        "userRole",
        "teamType",
        "templateId",
        "kind",
        "state",
        "teamSiteType",
        "memberRole",
        "tagType",
        "memberRoleString",
        "interest",
        "joiningenabled",
        "modality",
        "threadType",
        "threadSubType",
        "spaceTypes",
        "accessType",
        "folderType",
        "streamType",
        "errorCode",
        "code",
    ];
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
            if SAFE.contains(&key)
                || std::env::var("KELMA_SHOW").is_ok_and(|w| w.split(',').any(|k| k == key)) =>
        {
            println!("{pad}{key}: {s:?}")
        }
        Value::String(s) => println!("{pad}{key}: string({})", s.len()),
        Value::Number(n) => println!("{pad}{key}: {n}"),
        Value::Bool(b) => println!("{pad}{key}: {b}"),
        Value::Null => println!("{pad}{key}: null"),
    }
}

fn sandbox_file() -> Result<String> {
    std::env::var("KELMA_SANDBOX_FILE").context("set KELMA_SANDBOX_FILE to the sandbox ids file")
}

fn load_sandbox() -> Result<Value> {
    let path = sandbox_file()?;
    let s = std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
    Ok(serde_json::from_str(&s)?)
}

fn save_sandbox(v: &Value) -> Result<()> {
    std::fs::write(sandbox_file()?, serde_json::to_string_pretty(v)?)?;
    Ok(())
}

/// The team a write targets: `--team <thread id> --group <group id>`, else the sandbox.
fn target(args: &[String]) -> Result<TeamRef> {
    let opt = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    if let (Some(thread_id), Some(group_id)) = (opt("--team"), opt("--group")) {
        return Ok(TeamRef {
            thread_id,
            group_id,
        });
    }
    let s = load_sandbox()?;
    Ok(TeamRef {
        thread_id: s["team"]["thread_id"]
            .as_str()
            .context("no sandbox team")?
            .into(),
        group_id: s["team"]["group_id"]
            .as_str()
            .context("no sandbox team")?
            .into(),
    })
}

/// Positional arguments without the `--flag value` pairs and bare flags.
fn positional(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if matches!(
            a.as_str(),
            "--team" | "--group" | "--desc" | "--name" | "--discoverable"
        ) {
            skip = true;
        } else if !a.starts_with("--") {
            out.push(a.as_str());
        }
    }
    out
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

fn desc(args: &[String]) -> String {
    args.iter()
        .position(|a| a == "--desc")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_default()
}

fn role_of(s: &str) -> Result<Role> {
    Ok(match s {
        "owner" => Role::Owner,
        "member" => Role::Member,
        "guest" => Role::Guest,
        other => bail!("role must be owner, member or guest, not {other}"),
    })
}

/// Which of our teams is the sandbox (matched by group id).
fn is_sandbox(sb: &Value, team: &t::TeamEntry) -> bool {
    sb["team"]["group_id"].as_str() == Some(team.group_id.as_str())
}

fn print_channel(c: &t::ChannelEntry) {
    println!(
        "  # {:<20} {:?}{}{}{}{}{}{} {}",
        c.name,
        c.kind,
        if c.is_general { " general" } else { "" },
        if c.is_shown { "" } else { " hidden" },
        if c.is_pinned { " pinned" } else { "" },
        if c.is_muted { " muted" } else { "" },
        if c.is_archived { " archived" } else { "" },
        if c.is_deleted { " deleted" } else { "" },
        c.id
    );
}

async fn list(client: &Client) -> Result<()> {
    let teams = client.teams_and_channels().await?;
    let sb = load_sandbox().unwrap_or(Value::Null);
    println!("{} teams", teams.len());
    for team in &teams {
        let channels = team.channels.len();
        let hidden = team.channels.iter().filter(|c| !c.is_shown).count();
        if is_sandbox(&sb, team) {
            println!(
                "{} ({:?}, {:?}, {} members, {} owners, {} channels, {} hidden){}",
                team.name,
                team.access,
                team.role,
                team.member_count,
                team.owner_count,
                channels,
                hidden,
                if team.is_archived { " archived" } else { "" }
            );
            if !team.is_shown {
                println!("  (team hidden)");
            }
            team.channels.iter().for_each(print_channel);
        } else {
            println!(
                "team: {:?}, {:?}, {} members, {} channels ({} hidden, {} pinned, {} private){}",
                team.access,
                team.role,
                team.member_count,
                channels,
                hidden,
                team.channels.iter().filter(|c| c.is_pinned).count(),
                team.channels
                    .iter()
                    .filter(|c| c.kind == ChannelKind::Private)
                    .count(),
                if team.is_archived { " archived" } else { "" }
            );
        }
    }
    Ok(())
}

/// `team probe <mt|csa|graph|yammer> <path or url>`: GET and print the shape.
async fn probe(client: &Client, which: &str, path: &str) -> Result<()> {
    let mut path = path.to_string();
    if path.contains('{') {
        // `{team}`, `{group}` and `{chan}` stand for the sandbox team's thread id, group id and
        // its second channel, so no id is ever typed or printed.
        let s = load_sandbox()?;
        let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        path = path
            .replace(
                "{team}",
                &enc(s["team"]["thread_id"].as_str().unwrap_or_default()),
            )
            .replace(
                "{group}",
                s["team"]["group_id"].as_str().unwrap_or_default(),
            )
            .replace(
                "{chan}",
                &enc(s["channels"]["tests"].as_str().unwrap_or_default()),
            );
    }
    let v = match which {
        "mt" => {
            let url = format!("{}{path}", client.middle_tier().await);
            client.json("probe", Method::GET, &url, None).await?
        }
        "csa" => client.json("probe", Method::GET, &path, None).await?,
        "graph" => {
            client
                .json_as(
                    "probe",
                    &teams_proto::broker::Audience::Graph,
                    Method::GET,
                    &path,
                    None,
                )
                .await?
        }
        "yammer" => {
            client
                .json_as(
                    "probe",
                    &teams_proto::broker::Audience::Yammer,
                    Method::GET,
                    &path,
                    None,
                )
                .await?
        }
        other => bail!("unknown backend {other}"),
    };
    shape(&v, 0, "$");
    Ok(())
}

/// `team raw <METHOD> <mt|fabric|chat|csa> <path> [json]`: discovery writes. The path must name
/// the sandbox team, one of its channels or its group, so a real team can never be hit.
async fn raw(client: &Client, args: &[String]) -> Result<()> {
    let [method, backend, path] = args.get(..3).unwrap_or_default() else {
        bail!("usage: team raw <METHOD> <mt|fabric|chat|csa> <path> [json]")
    };
    let sb = load_sandbox()?;
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let mut ids: Vec<String> = vec![
        sb["team"]["thread_id"].as_str().unwrap_or("~").into(),
        sb["team"]["group_id"].as_str().unwrap_or("~").into(),
    ];
    ids.extend(
        sb["channels"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, v)| v.as_str().map(str::to_owned)),
    );
    let mut path = path.to_string();
    let backend = backend.as_str();
    let me = enc(&client.self_mri().await?);
    for (k, v) in [
        ("{team}", enc(&ids[0])),
        ("{group}", ids[1].clone()),
        ("{me}", me),
        (
            "{tests}",
            enc(sb["channels"]["tests"].as_str().unwrap_or("~")),
        ),
    ] {
        path = path.replace(k, &v);
    }
    let me_raw = client.self_mri().await?;
    let join_code = if path.contains("joinByCode") {
        // Only the sandbox team's own code may be tried, never a guessed one.
        let url = format!(
            "{}/beta/teams/{}/joinCode",
            client.middle_tier().await,
            enc(&ids[0])
        );
        client
            .request(Method::GET, &url, None, &[])
            .await?
            .1
            .trim()
            .to_string()
    } else {
        String::new()
    };
    let body: Option<Value> = args
        .get(3)
        .map(|s| {
            serde_json::from_str(
                &s.replace("{code}", &join_code)
                    .replace("{me}", &me_raw)
                    .replace("{tests}", sb["channels"]["tests"].as_str().unwrap_or("~")),
            )
        })
        .transpose()?;
    let body_text = body.as_ref().map(Value::to_string).unwrap_or_default();
    let names = |hay: &str| {
        ids.iter()
            .any(|i| hay.contains(i.as_str()) || hay.contains(&enc(i)))
    };
    let own_code =
        path.contains("joinByCode") && !join_code.is_empty() && body_text.contains(&join_code);
    if !names(&path) && !(path.contains("pinnedChannels") && names(&body_text)) && !own_code {
        bail!("refusing: the path names no sandbox id");
    }
    let method = Method::from_bytes(method.as_bytes())?;
    let (status, text) = match backend {
        "mt" => {
            let url = format!("{}{path}", client.middle_tier().await);
            client.request(method, &url, body.as_ref(), &[]).await?
        }
        "chat" => {
            let url = format!("{}{path}", client.chat_service().await);
            client.request(method, &url, body.as_ref(), &[]).await?
        }
        "csa" => client.request(method, &path, body.as_ref(), &[]).await?,
        "fabric" => {
            let me = client.self_mri().await?;
            let tid = client.tenant_id().await?;
            let anchor = format!("OID:{}@{tid}", me.trim_start_matches("8:orgid:"));
            let url = format!(
                "https://teams.microsoft.com/fabric/{}/templates/api/{path}",
                client.geo().await
            );
            client
                .request_as(
                    &teams_proto::broker::Audience::Templates,
                    method,
                    &url,
                    body.as_ref(),
                    &[("X-AnchorMailbox", anchor.as_str())],
                )
                .await?
        }
        other => bail!("unknown backend {other}"),
    };
    println!("HTTP {status}");
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if status < 300 => shape(&v, 0, "$"),
        _ => println!("{}", text.chars().take(500).collect::<String>()),
    }
    Ok(())
}

/// Looks a channel up in our list and checks it belongs to the target team, so a typo can
/// never reach another team.
async fn resolve(
    client: &Client,
    team: &TeamRef,
    channel_id: &str,
) -> Result<(t::ChannelEntry, t::ChannelRef)> {
    let teams = client.teams_and_channels().await?;
    let (owner, ch) = t::find_channel(&teams, channel_id).context("no such channel in the list")?;
    if owner.group_id != team.group_id {
        bail!("that channel is not in the target team");
    }
    Ok((ch.clone(), t::ChannelRef::from_entry(ch)))
}

fn on_off(s: &str) -> Result<bool> {
    match s {
        "on" | "true" | "yes" => Ok(true),
        "off" | "false" | "no" => Ok(false),
        other => bail!("expected on or off, not {other}"),
    }
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

async fn channel_cmd(client: &Client, verb: &str, rest: &[String], pos: &[&str]) -> Result<()> {
    let team = target(rest)?;
    let need = |n: usize, usage: &str| -> Result<()> {
        if pos.len() < n {
            bail!("usage: team channel {usage}")
        }
        Ok(())
    };
    match verb {
        "new" => {
            need(
                2,
                "new <standard|private|shared> <name> [--threaded] [--desc d]",
            )?;
            let (kind, name) = (pos[0], pos[1]);
            let id = match kind {
                "standard" => {
                    client
                        .create_standard_channel(&team, name, &desc(rest), has(rest, "--threaded"))
                        .await?
                }
                "private" | "shared" => {
                    let k = if kind == "private" {
                        ChannelKind::Private
                    } else {
                        ChannelKind::Shared
                    };
                    client
                        .create_private_channel(&team, name, &desc(rest), k)
                        .await?
                }
                other => bail!("channel kind must be standard, private or shared, not {other}"),
            };
            println!("created channel {id}");
        }
        "edit" => {
            need(1, "edit <id> [--name n] [--desc d]")?;
            let (_, r) = resolve(client, &team, pos[0]).await?;
            let d = has(rest, "--desc").then(|| desc(rest));
            let n = opt(rest, "--name");
            client
                .edit_channel(&team, &r, n.as_deref(), d.as_deref())
                .await?;
            println!("edited");
        }
        "rm" => {
            need(1, "rm <id>")?;
            let (_, r) = resolve(client, &team, pos[0]).await?;
            client.delete_channel(&team, &r).await?;
            println!("deleted");
        }
        "restore" => {
            need(1, "restore <id>")?;
            client.restore_channel(&team.thread_id, pos[0]).await?;
            println!("restored");
        }
        "deleted" | "archived" => {
            let list = client.channels_in_state(&team.thread_id, verb).await?;
            println!("{} {verb} channels", list.len());
            for c in &list {
                println!("  # {} {}", c.name, c.id);
            }
        }
        "archive" | "unarchive" => {
            need(1, "archive <id>")?;
            resolve(client, &team, pos[0]).await?;
            client
                .set_archival(pos[0], false, verb == "archive")
                .await?;
            println!("{verb}d");
        }
        "show" | "hide" => {
            need(1, "show <id>")?;
            let (c, r) = resolve(client, &team, pos[0]).await?;
            client
                .set_channel_shown(&r, c.is_general, verb == "show")
                .await?;
            println!("{verb} done");
        }
        "default" => {
            need(2, "default <id> <on|off>")?;
            resolve(client, &team, pos[0]).await?;
            client
                .set_channel_default_shown(pos[0], on_off(pos[1])?)
                .await?;
            println!("default set");
        }
        "pin" | "unpin" => {
            need(1, "pin <id>")?;
            resolve(client, &team, pos[0]).await?;
            let p = client.set_channel_pinned(pos[0], verb == "pin").await?;
            println!("{} pinned channels", p.channel_ids.len());
        }
        "pinned" => {
            let p = client.pinned_channels().await?;
            println!("{} pinned channels", p.channel_ids.len());
        }
        "moderate" => {
            need(
                2,
                "moderate <id> show | moderate <id> <posts> <replies> <pins> [--no-bots] [--no-connectors]",
            )?;
            resolve(client, &team, pos[0]).await?;
            if pos[1] == "show" {
                println!("{:?}", client.moderation(pos[0]).await?);
            } else {
                need(
                    4,
                    "moderate <id> <Everyone|Moderators> <Everyone|Moderators> <Everyone|Moderators>",
                )?;
                let m = Moderation {
                    post_permissions: pos[1].into(),
                    reply_permissions: pos[2].into(),
                    pin_permissions: pos[3].into(),
                    allow_bots: !has(rest, "--no-bots"),
                    allow_connectors: !has(rest, "--no-connectors"),
                };
                client.set_moderation(pos[0], &m).await?;
                println!("moderation set");
            }
        }
        "email" => {
            need(2, "email <show|anyone|members|domains|rm> <id> [domain..]")?;
            let (action, id) = (pos[0], pos[1]);
            resolve(client, &team, id).await?;
            match action {
                "show" => match client.channel_email(id).await? {
                    Some(e) => println!(
                        "address set ({} chars), senders {:?}, {} domains",
                        e.address.len(),
                        e.senders,
                        e.domains.len()
                    ),
                    None => println!("no address"),
                },
                "rm" => {
                    client.remove_channel_email(id).await?;
                    println!("removed");
                }
                "anyone" | "members" | "domains" => {
                    let senders = match action {
                        "anyone" => t::EmailSenders::Anyone,
                        "members" => t::EmailSenders::Members,
                        _ => t::EmailSenders::Domains,
                    };
                    let domains: Vec<String> = pos[2..].iter().map(|s| s.to_string()).collect();
                    let exists = client.channel_email(id).await?.is_some();
                    let e = client
                        .set_channel_email(id, senders, &domains, exists)
                        .await?;
                    println!("set, address present: {}", e.is_some());
                }
                other => bail!("unknown email action {other}"),
            }
        }
        "notify" => {
            need(2, "notify <show|banner|feed|off> <id> [--replies]")?;
            let (action, id) = (pos[0], pos[1]);
            resolve(client, &team, id).await?;
            if action == "show" {
                println!("{:?}", client.channel_notifications(id).await?);
            } else {
                let level = match action {
                    "banner" => t::NotifyLevel::BannerAndFeed,
                    "feed" => t::NotifyLevel::Feed,
                    "off" => t::NotifyLevel::Off,
                    other => bail!("unknown level {other}"),
                };
                let n = t::ChannelNotify {
                    level,
                    include_replies: has(rest, "--replies"),
                };
                client.set_channel_notifications(id, n).await?;
                println!("notifications set");
            }
        }
        "members" => {
            need(2, "members <ls|add|role|rm> <id> [mri] [role]")?;
            let (action, id) = (pos[0], pos[1]);
            let (c, r) = resolve(client, &team, id).await?;
            match action {
                "ls" => print_roster(&client.roster(&c.id).await?),
                "add" | "role" => {
                    need(4, "members add <id> <mri> <owner|member>")?;
                    client
                        .set_private_channel_members(
                            &team,
                            &r,
                            &[(pos[2].to_string(), role_of(pos[3])?)],
                        )
                        .await?;
                    println!("{action} done");
                }
                "rm" => {
                    need(3, "members rm <id> <mri>")?;
                    client
                        .remove_private_channel_member(&team, &r, pos[2])
                        .await?;
                    println!("removed");
                }
                other => bail!("unknown members action {other}"),
            }
        }
        other => bail!("unknown channel command {other}"),
    }
    Ok(())
}

fn print_roster(roster: &[t::RosterMember]) {
    let count = |r: Role| roster.iter().filter(|m| !m.is_bot && m.role == r).count();
    println!(
        "{} people ({} owners, {} members, {} guests), {} bots",
        roster.iter().filter(|m| !m.is_bot).count(),
        count(Role::Owner),
        count(Role::Member),
        count(Role::Guest),
        roster.iter().filter(|m| m.is_bot).count()
    );
}

async fn tag_cmd(client: &Client, verb: &str, rest: &[String], pos: &[&str]) -> Result<()> {
    let team = target(rest)?;
    let tid = team.thread_id.as_str();
    let find = |tags: &[Tag], name: &str| -> Result<Tag> {
        tags.iter()
            .find(|t| t.name == name || t.id == name)
            .cloned()
            .context("no such tag")
    };
    match verb {
        "ls" => {
            let tags = client.tags(tid).await?;
            println!("{} tags", tags.len());
            for t in &tags {
                println!(
                    "  {} ({} members{}) mention mri {}",
                    t.name,
                    t.member_count,
                    if t.is_member { ", you" } else { "" },
                    t.mri()
                );
            }
        }
        "new" => {
            let name = pos.first().context("usage: team tag new <name> [mri..]")?;
            let members: Vec<String> = pos[1..].iter().map(|s| s.to_string()).collect();
            let tag = client.create_tag(tid, name, &desc(rest), &members).await?;
            println!("created tag {} ({} members)", tag.mri(), tag.member_count);
        }
        "rename" => {
            let [name, new] = pos[..] else {
                bail!("usage: team tag rename <name> <new name> [--desc d]")
            };
            let tag = find(&client.tags(tid).await?, name)?;
            let d = has(rest, "--desc").then(|| desc(rest));
            client.edit_tag(tid, &tag, Some(new), d.as_deref()).await?;
            println!("renamed");
        }
        "rm" => {
            let name = pos.first().context("usage: team tag rm <name>")?;
            let tag = find(&client.tags(tid).await?, name)?;
            client.delete_tag(tid, &tag.id).await?;
            println!("deleted");
        }
        "members" => {
            let name = pos.first().context("usage: team tag members <name>")?;
            let tag = find(&client.tags(tid).await?, name)?;
            println!("{} members", client.tag_members(tid, &tag.id).await?.len());
        }
        "add" | "remove" => {
            let [name, mri] = pos[..] else {
                bail!("usage: team tag {verb} <name> <mri>")
            };
            let tag = find(&client.tags(tid).await?, name)?;
            if verb == "add" {
                client
                    .add_to_tags(tid, &[mri.to_string()], std::slice::from_ref(&tag.name))
                    .await?;
            } else {
                client
                    .remove_from_tags(tid, &[mri.to_string()], std::slice::from_ref(&tag.id))
                    .await?;
            }
            println!("{verb} done");
        }
        other => bail!("unknown tag command {other}"),
    }
    Ok(())
}

pub async fn run(client: &Client, args: &[String]) -> Result<()> {
    let arg = |n: usize| args.get(n).map(String::as_str);
    let rest = if args.len() > 2 { &args[2..] } else { &[] };
    let pos = positional(rest);
    match (arg(0), arg(1)) {
        (Some("probe"), Some(which)) => {
            probe(client, which, arg(2).context("missing path")?).await?
        }
        (Some("raw"), _) => raw(client, &args[1..]).await?,
        (Some("tabs"), _) => {
            // Read only: the sandbox channel's tabs (names, apps and the host they open).
            let s = load_sandbox()?;
            let group = s["team"]["group_id"].as_str().unwrap_or_default();
            let chan = s["channels"]["tests"].as_str().unwrap_or_default();
            let tabs = client.channel_tabs(group, chan).await?;
            println!("{} tab(s) besides Files", tabs.len());
            for t in &tabs {
                let host = url::Url::parse(&t.open_url)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_string))
                    .unwrap_or_default();
                println!("  {} ({}), opens {host}", t.name, t.app_name);
            }
        }
        (Some("ls"), _) => list(client).await?,
        (Some("sandbox"), Some("create")) => {
            if load_sandbox().is_ok() {
                bail!("a sandbox team is already recorded in {}", sandbox_file()?);
            }
            let name = "kelma-sandbox";
            let check = client.validate_team_name(name).await?;
            println!("name check:");
            shape(&check, 2, "answer");
            let team = client
                .create_team(name, "kelma test team", Access::Private)
                .await?;
            save_sandbox(
                &json!({"team": {"thread_id": team.thread_id, "group_id": team.group_id}, "channels": {}}),
            )?;
            println!("created the sandbox team and saved its ids");
        }
        (Some("sandbox"), Some("show")) => {
            let sb = load_sandbox()?;
            println!(
                "team ids: {}, channels: {}",
                sb["team"].as_object().map_or(0, |o| o.len()),
                sb["channels"].as_object().map_or(0, |o| o.len())
            );
        }
        // Teams: discover and join.
        (Some("discover"), _) => {
            let access = if has(args, "--private") {
                Access::Private
            } else {
                Access::Public
            };
            let found = match arg(1).filter(|a| !a.starts_with("--")) {
                Some(q) => client.find_teams(q, access, "").await?,
                None => client.suggested_teams(access).await?,
            };
            println!("{} teams", found.len());
            for d in &found {
                println!("  {} members, {}", d.members, d.access);
            }
        }
        (Some("join"), Some(group)) => {
            let v = client.join_team(group).await?;
            shape(&v, 0, "$");
        }
        (Some("join-code"), Some(code)) => {
            let info = client
                .join_team_with_code(code, has(args, "--public-only"))
                .await?;
            println!("{info:?}");
        }
        (Some("code"), Some(v @ ("get" | "reset" | "rm"))) => {
            let team = target(rest)?;
            match v {
                "get" => println!(
                    "code set: {}",
                    client.team_join_code(&team.thread_id).await?.is_some()
                ),
                "reset" => println!(
                    "new code of {} chars",
                    client.reset_team_join_code(&team.thread_id).await?.len()
                ),
                _ => {
                    client.remove_team_join_code(&team.thread_id).await?;
                    println!("code removed");
                }
            }
        }
        // Teams: manage.
        (Some("new"), Some(name)) => {
            let access = if has(rest, "--public") {
                Access::Public
            } else {
                Access::Private
            };
            let team = client.create_team(name, &desc(rest), access).await?;
            println!("created {}", team.thread_id);
        }
        (Some("rm"), _) => {
            let team = target(&args[1..])?;
            client.delete_team(&team.thread_id).await?;
            println!("deleted");
        }
        (Some("edit"), _) => {
            let all = &args[1..];
            let team = target(all)?;
            let d = has(all, "--desc").then(|| desc(all));
            let disc = opt(all, "--discoverable").map(|s| on_off(&s)).transpose()?;
            client
                .edit_team(
                    &team.thread_id,
                    opt(all, "--name").as_deref(),
                    d.as_deref(),
                    disc,
                )
                .await?;
            println!("edited");
        }
        (Some("settings"), Some("show")) => {
            let team = target(rest)?;
            let s = client.team_settings(&team.thread_id).await?;
            for who in ["userSettings", "guestSettings"] {
                let m = s[who].as_object();
                println!(
                    "{who}: {} settings, {} on",
                    m.map_or(0, |o| o.len()),
                    m.map_or(0, |o| o
                        .values()
                        .filter(|v| **v == Value::Bool(true))
                        .count())
                );
            }
        }
        (Some("settings"), Some("set")) => {
            let team = target(rest)?;
            let [who, key, value] = pos[..] else {
                bail!("usage: team settings set <user|guest> <key> <on|off|number>")
            };
            let v: Value = match value.parse::<i64>() {
                Ok(n) => json!(n),
                Err(_) => json!(on_off(value)?),
            };
            let one = json!({ key: v });
            match who {
                "user" => {
                    client
                        .set_team_settings(&team.thread_id, Some(&one), None)
                        .await?
                }
                "guest" => {
                    client
                        .set_team_settings(&team.thread_id, None, Some(&one))
                        .await?
                }
                other => bail!("user or guest, not {other}"),
            }
            println!("setting changed");
        }
        (Some(v @ ("archive" | "unarchive")), _) => {
            let team = target(&args[1..])?;
            if has(&args[1..], "--flag") {
                client
                    .set_archival(&team.thread_id, true, v == "archive")
                    .await?;
            } else if v == "archive" {
                client.archive_team(&team.thread_id).await?;
            } else {
                client.unarchive_team(&team.thread_id).await?;
            }
            println!("{v}d");
        }
        (Some(v @ ("show" | "hide")), _) => {
            let team = target(&args[1..])?;
            client.set_team_shown(&team.thread_id, v == "show").await?;
            println!("{v} done");
        }
        (Some("channel"), Some(verb)) => channel_cmd(client, verb, rest, &pos).await?,
        (Some("tag"), Some(verb)) => tag_cmd(client, verb, rest, &pos).await?,
        (Some("members"), Some("ls")) => {
            let id = match pos.first() {
                Some(id) => id.to_string(),
                None => target(rest)?.thread_id,
            };
            print_roster(&client.roster(&id).await?);
        }
        (Some("members"), Some(v @ ("add" | "role"))) => {
            let team = target(rest)?;
            let [mri, role] = pos[..] else {
                bail!("usage: team members {v} <mri> <owner|member|guest>")
            };
            client
                .set_team_members(&team, &[(mri.to_string(), role_of(role)?)])
                .await?;
            println!("{v} done");
        }
        (Some("members"), Some("rm")) => {
            let team = target(rest)?;
            let mri = pos.first().context("usage: team members rm <mri>")?;
            client.remove_team_member(&team, mri).await?;
            println!("removed");
        }
        // Viva Engage.
        (Some("community"), Some("ls")) => {
            let list = client.communities().await?;
            println!("{} communities", list.len());
            for c in &list {
                println!(
                    "  id {} {} members{}",
                    c.id,
                    c.members,
                    if c.is_company_wide {
                        ", company-wide"
                    } else {
                        ""
                    }
                );
            }
        }
        (Some("community"), Some("feed")) => {
            let id: u64 = pos
                .first()
                .context("usage: team community feed <id> [n]")?
                .parse()?;
            let n: u32 = pos.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
            let posts = client.community_feed(id, n, None).await?;
            println!("{} posts", posts.len());
            for p in &posts {
                println!(
                    "  {} replies-to {:?} {} chars, {} likes, {} attachments",
                    p.id,
                    p.replied_to_id,
                    p.text.chars().count(),
                    p.likes,
                    p.attachments
                );
            }
        }
        (Some("storyline-settings"), _) => {
            // Read only: one Engage GraphQL query, nothing is posted.
            let st = client.storyline_settings().await?;
            println!(
                "storyline: access {}, can post {}, announcements {}, stories {}",
                st.can_access_storyline,
                st.can_post,
                st.can_post_announcement,
                st.can_access_stories
            );
        }
        (Some("storyline"), Some(text)) => {
            // Refused in teams-proto until the Engage GraphQL route is verified (plan T2.9).
            let _ = (text, has(args, "--yes"));
            bail!("storyline posting is disabled: see docs/research/storyline.md");
        }
        _ => bail!(
            "usage: team <ls|discover|join|join-code|code|new|rm|edit|settings|archive|unarchive|show|hide|channel|tag|members|community|storyline|storyline-settings|sandbox|probe|raw> …"
        ),
    }
    Ok(())
}
