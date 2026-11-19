//! `kelma-cli`: exercise the protocol from a terminal.
//!
//! ```text
//! kelma-cli login                 sign in with a device code
//! kelma-cli chats [n]             list recent conversations
//! kelma-cli teams                 list teams and channels
//! kelma-cli activity [n]          list the newest Activity feed items
//! kelma-cli read <conv> [n]       print the last n messages
//! kelma-cli send <conv> <text>    send a message (use 48:notes for your own notes)
//! kelma-cli react <conv> <msg> <key>
//! kelma-cli calendar [days]       count upcoming events per day and per show-as (read-only)
//! kelma-cli calendar <check>      live checks of the calendar data layer (calendars, rooms, mailbox, schedule, ...)
//! kelma-cli audiences             mint a token for every audience (prints no tokens)
//! kelma-cli people <query>        search people (prints the count first)
//! kelma-cli presence [mri...]     presence of the given people, default yourself
//! kelma-cli card [mri]            profile card of a person, default yourself
//! kelma-cli horizons <conv>       who has read up to which message in a chat
//! kelma-cli drive <ls [path]|search q|recent|shared|favorites|recordings|libraries|sync-help|bin|mkdir name|rm id|selftest>
//! kelma-cli tasks [todo <n>|plan <n>]  To Do lists and Planner plans (read-only)
//! kelma-cli flows [runs <n> [top]]  your Workflows and their run history (read-only)
//! kelma-cli onenote [<notebook> [<section> [<page>]]]  read OneNote notebooks, sections, pages
//! kelma-cli lists [<site> [<list>]]  SharePoint lists of your teams' sites (read-only)
//! kelma-cli store <bar|more|home|search|app|mine|org|settings|install|uninstall|update|pin|unpin|move|probe>  app bar and Store (writes are dry runs)
//! kelma-cli team <ls|discover|join|code|new|edit|settings|archive|channel|tag|members|community|…>
//!                                 teams and channels (writes go to the sandbox team, see team.rs)
//! kelma-cli tail                  print live events until Ctrl+C
//! kelma-cli diag [--log F] [--out P]  write a local, redacted problem-report bundle (nothing is uploaded)
//! kelma-cli logout                remove the stored refresh token
//! ```

mod apps;
mod drive;
mod store;
mod team;

use anyhow::{Context, Result, bail};
use session::Paths;
use teams_proto::trouter::TrouterEvent;
use teams_proto::{Client, model::Event};

fn fmt_time(ms: i64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60)
}

fn preview(html: &str) -> String {
    let t = render::plain_text(&render::parse_teams_html(html));
    let one: String = t.replace('\n', " ");
    if one.chars().count() > 80 {
        format!("{}…", one.chars().take(80).collect::<String>())
    } else {
        one
    }
}

async fn self_mri(client: &Client, cfg: &session::Config) -> Result<String> {
    match &cfg.self_mri {
        Some(m) => Ok(m.clone()),
        None => Ok(client.self_mri().await?),
    }
}

async fn connect(paths: &Paths) -> Result<(Client, session::Config)> {
    let cfg = paths.load_config()?;
    let sess = session::stored_session(&cfg)?.context("not signed in: run `kelma-cli login`")?;
    let client = Client::resume(sess).await?;
    session::persist(&cfg, &client).await?;
    Ok((client, cfg))
}

/// `kelma-cli calendar <check>`: the calendar data layer against the live service. Prints
/// counts and key names only, never titles, people or addresses. `write-check` and
/// `calendar-check` are the only writes: a private appointment (and a calendar) made and
/// removed again.
async fn calendar_check(client: &Client, args: &[String]) -> Result<()> {
    use calendar as cal;
    let now = teams_proto::time::now_ms();
    let day = 86_400_000;
    match args.first().map(String::as_str) {
        Some("calendars") => {
            let list = cal::calendars(client).await?;
            let groups = cal::calendar_groups(client).await?;
            println!(
                "{} calendars ({} default, {} editable, {} with a hex color), {} groups",
                list.len(),
                list.iter().filter(|c| c.is_default).count(),
                list.iter().filter(|c| c.can_edit).count(),
                list.iter().filter(|c| c.hex_color.is_some()).count(),
                groups.len()
            );
            if let Some(g) = groups.first() {
                let n = cal::calendars_in_group(client, &g.id).await?.len();
                println!("first group holds {n} calendars");
            }
            if let Some(c) = list.iter().find(|c| c.is_default) {
                let perms = cal::calendar_permissions(client, &c.id).await?;
                println!("default calendar has {} permission entries", perms.len());
            }
        }
        Some("rooms") => {
            let lists = cal::room_lists(client).await?;
            println!("{} room lists", lists.len());
            println!(
                "{} rooms in the tenant",
                cal::rooms(client, "").await?.len()
            );
            if let Some(l) = lists.first() {
                let rooms = cal::rooms(client, &l.email).await?;
                println!(
                    "first list: {} rooms, {} with capacity, {} with a building",
                    rooms.len(),
                    rooms.iter().filter(|r| r.capacity.is_some()).count(),
                    rooms.iter().filter(|r| r.building.is_some()).count()
                );
            }
        }
        Some("mailbox") => {
            let s = cal::mailbox_settings(client).await?;
            println!(
                "time zone set: {}, work days: {}, work hours {:02}:{:02}-{:02}:{:02}, date format set: {}, time format set: {}",
                !s.time_zone.is_empty(),
                s.working_hours.days.len(),
                s.working_hours.start_min / 60,
                s.working_hours.start_min % 60,
                s.working_hours.end_min / 60,
                s.working_hours.end_min % 60,
                !s.date_format.is_empty(),
                !s.time_format.is_empty()
            );
        }
        Some("categories") => {
            let c = cal::categories(client).await?;
            println!(
                "{} categories, {} with a color",
                c.len(),
                c.iter().filter(|c| c.color_preset.is_some()).count()
            );
        }
        Some("schedule") => {
            let me = client
                .json_as(
                    "me",
                    &teams_proto::broker::Audience::Graph,
                    reqwest::Method::GET,
                    "https://graph.microsoft.com/v1.0/me?$select=mail,userPrincipalName",
                    None,
                )
                .await?;
            let email = me["mail"]
                .as_str()
                .or(me["userPrincipalName"].as_str())
                .context("no mail address on /me")?
                .to_string();
            let start = now - now.rem_euclid(day);
            let schedules =
                cal::get_schedule(client, std::slice::from_ref(&email), start, start + day, 30)
                    .await?;
            for s in &schedules {
                println!(
                    "my schedule today: {} slots, {} busy, {} items, error: {}",
                    s.availability.len(),
                    s.availability
                        .iter()
                        .filter(|a| **a == cal::Availability::Busy)
                        .count(),
                    s.items.len(),
                    s.error.is_some()
                );
            }
            match cal::person_view(client, &email, start, start + day, 30).await? {
                cal::PersonView::Events(e) => {
                    println!("my calendar via /users/{{upn}}: {} events", e.len())
                }
                cal::PersonView::FreeBusyOnly(_) => {
                    println!("my calendar via /users/{{upn}}: free/busy fallback")
                }
            }
            let found = cal::find_meeting_times(
                client,
                &cal::FindTimes {
                    required: vec![cal::Person::new("", &email)],
                    optional: vec![],
                    duration_minutes: 30,
                    window_start_ms: start + 2 * day,
                    window_end_ms: start + 3 * day,
                    max_candidates: 5,
                    min_attendee_percentage: 100,
                    work_hours_only: true,
                },
            )
            .await?;
            println!(
                "find meeting times (only me, in 2 days): {} suggestions, empty reason set: {}",
                found.suggestions.len(),
                found.empty_reason.is_some()
            );
        }
        Some("probe-outlook") => {
            // GET only, prints the status and the top-level key names, never values.
            let path = args
                .get(1)
                .context("probe-outlook <path under https://outlook.office.com/>")?;
            let url = format!(
                "https://outlook.office.com/{}",
                path.trim_start_matches('/')
            );
            let (status, text) = client
                .request_as(
                    &teams_proto::broker::Audience::Outlook,
                    reqwest::Method::GET,
                    &url,
                    None,
                    &[],
                )
                .await?;
            let keys: Vec<String> = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
                .unwrap_or_default();
            let code = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["error"]["code"].as_str().map(String::from));
            println!(
                "HTTP {status}, {} bytes, keys {keys:?}, error code {code:?}",
                text.len()
            );
        }
        Some("worklocation") => {
            let w = cal::work_location(client).await?;
            println!("work location: {w:?}");
        }
        Some("detail") => {
            let events = cal::Calendar::view(client, now, now + 7 * day).await?;
            let Some(e) = events.iter().find(|e| !e.attendees.is_empty()) else {
                println!("no upcoming event with attendees to read");
                return Ok(());
            };
            let d = cal::event_detail(client, &e.id).await?;
            println!(
                "detail: body {} bytes, {} attachments, counts {:?}, conference id set: {}, thread id from join url: {}",
                d.body.len(),
                d.attachments.len(),
                d.counts,
                d.dial_in.conference_id.is_some(),
                e.join_url
                    .as_deref()
                    .and_then(cal::meeting_chat_thread_id)
                    .is_some()
            );
            if let Some(master) = &e.series_master_id {
                let inst = cal::instances(client, master, now, now + 30 * day).await?;
                println!("series has {} occurrences in 30 days", inst.len());
            }
        }
        Some("ics") => {
            let events = cal::Calendar::view(client, now, now + 7 * day).await?;
            let ics = cal::to_ics(&events, now);
            println!(
                "{} events exported to {} bytes of .ics ({} VEVENT)",
                events.len(),
                ics.len(),
                ics.matches("BEGIN:VEVENT").count()
            );
        }
        Some("write-check") => {
            let start = now + 30 * day;
            let start = start - start.rem_euclid(day) + 12 * 3_600_000;
            let wall = |ms: i64| cal::format_iso_ms(ms)[..19].to_string();
            let new = cal::NewEvent {
                subject: "kelma test".into(),
                start: wall(start),
                end: wall(start + 1_800_000),
                is_teams_meeting: false,
                sensitivity: cal::Sensitivity::Private,
                ..cal::NewEvent::default()
            };
            let created = cal::create_event(client, &new).await?;
            println!(
                "created: subject ok {}, private {}, attendees {}, online {}",
                created.subject == "kelma test",
                created.sensitivity_private,
                created.attendees.len(),
                created.is_online_meeting
            );
            let mut changed = new.clone();
            changed.subject = "kelma test (updated)".into();
            let outcome = cal::update_event(client, &created.id, &changed).await;
            match &outcome {
                Ok(u) => println!(
                    "updated: subject changed {}",
                    u.subject == "kelma test (updated)"
                ),
                Err(e) => println!("update failed: {e}"),
            }
            cal::delete_event(client, &created.id).await?;
            println!("deleted");
            let left = cal::Calendar::view(client, start - day, start + day)
                .await?
                .iter()
                .filter(|e| e.subject.starts_with("kelma test"))
                .count();
            println!("cleanup proof: {left} events named 'kelma test*' left in the window");
            outcome?;
        }
        Some("calendar-check") => {
            let made = cal::create_calendar(client, "kelma-test", "lightBlue").await?;
            println!("created calendar, color {}", made.color);
            cal::delete_calendar(client, &made.id).await?;
            let left = cal::calendars(client)
                .await?
                .iter()
                .filter(|c| c.name == "kelma-test")
                .count();
            println!("cleanup proof: {left} calendars named 'kelma-test' left");
        }
        other => bail!(
            "unknown calendar check {other:?}; try calendars, rooms, mailbox, schedule, worklocation, detail, ics, write-check, calendar-check"
        ),
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("KELMA_LOG"))
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = Paths::new()?;
    match args.first().map(String::as_str) {
        Some("login") => {
            let mut cfg = paths.load_config()?;
            let client = Client::sign_in_device_code(&cfg.tenant, &cfg.epid, |dc| {
                println!(
                    "Open {} and enter the code: {}",
                    dc.verification_url, dc.user_code
                );
            })
            .await?;
            session::persist(&cfg, &client).await?;
            let me = client.self_mri().await?;
            println!(
                "Signed in as {me} (chat service: {})",
                client.chat_service().await
            );
            cfg.self_mri = Some(me);
            paths.save_config(&cfg)?;
        }
        Some("diag") => {
            let out = diag::report_from_args(&paths, env!("CARGO_PKG_VERSION"), &args[1..])?;
            println!(
                "Wrote {}\nIt was made locally and nothing was uploaded. Tokens, emails, ids and \
                 tenant details are redacted; skim it before you share it.",
                out.display()
            );
        }
        Some("logout") => {
            let cfg = paths.load_config()?;
            session::delete_refresh_token(&cfg.account)?;
            println!("Signed out.");
        }
        Some("chats") => {
            let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
            let (client, _) = connect(&paths).await?;
            let convs = client.conversations(0, 100).await?;
            for c in convs.iter().take(n) {
                let title = c.topic.clone().unwrap_or_else(|| c.id.clone());
                let last = c
                    .last_message
                    .as_ref()
                    .map(|m| format!("{}: {}", m.sender_name, preview(&m.content)))
                    .unwrap_or_default();
                let unread = if c.last_activity_ms > c.read_horizon_ms {
                    "*"
                } else {
                    " "
                };
                println!(
                    "{unread} {:<9?} {:<40} {}\n    {}",
                    c.kind,
                    title,
                    fmt_time(c.last_activity_ms),
                    last
                );
                println!("    id: {}", c.id);
            }
        }
        Some("teams") => {
            let (client, _) = connect(&paths).await?;
            let snap = client.snapshot().await?;
            for t in &snap.teams {
                println!("{}", t.name);
                for ch in &t.channels {
                    println!("  # {:<30} {}", ch.name, ch.id);
                }
            }
            println!("({} chats in aggregator)", snap.chats.len());
        }
        Some("activity") => {
            let n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
            let (client, _) = connect(&paths).await?;
            let items = client.activity(n).await?;
            for i in &items {
                println!(
                    "{} {:<28} {} {}\n    in: {}  id: {}",
                    if i.is_read { " " } else { "*" },
                    format!("{:?}", i.kind),
                    fmt_time(i.timestamp_ms),
                    i.actor_name,
                    i.thread_topic,
                    i.id
                );
                if !i.preview.is_empty() {
                    println!("    {}", preview(&i.preview));
                }
            }
            println!("({} items)", items.len());
        }
        Some("read") => {
            let conv = args.get(1).context("usage: read <conversation id> [n]")?;
            let n: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
            let (client, _) = connect(&paths).await?;
            let page = client.messages(conv, None, n).await?;
            for m in page.messages.iter().filter(|m| m.is_chat_content()) {
                let flags = format!(
                    "{}{}",
                    if m.edited_ms.is_some() {
                        " (edited)"
                    } else {
                        ""
                    },
                    if m.deleted { " (deleted)" } else { "" }
                );
                let reacts: Vec<String> = m
                    .reactions
                    .iter()
                    .map(|r| format!("{}×{}", r.key, r.users.len()))
                    .collect();
                println!(
                    "[{}] {}{}: {}  {}",
                    fmt_time(m.compose_ms),
                    m.sender_name,
                    flags,
                    preview(&m.content),
                    reacts.join(" ")
                );
                for f in &m.files {
                    println!("      📎 {} {}", f.name, f.url);
                }
            }
            println!(
                "(id of last: {})",
                page.messages.last().map(|m| m.id.as_str()).unwrap_or("-")
            );
        }
        Some("raw") => {
            // Prints raw message JSON (structure discovery). Contains message content, never tokens.
            let conv = args.get(1).context("usage: raw <conversation id> [n]")?;
            let n: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
            let (client, _) = connect(&paths).await?;
            let url = format!(
                "{}/v1/users/ME/conversations/{}/messages?view=msnp24Equivalent|supportsMessageProperties&pageSize={n}&startTime=1",
                client.chat_service().await,
                teams_proto::chatsvc::encode_conversation(conv)
            );
            let v = client.json("raw", reqwest::Method::GET, &url, None).await?;
            println!("{}", serde_json::to_string_pretty(&v["messages"])?);
        }
        Some("csa-raw") => {
            // Prints the aggregator entry of one chat (structure discovery).
            let id = args.get(1).context("usage: csa-raw <conversation id>")?;
            let (client, _) = connect(&paths).await?;
            let url = "https://teams.microsoft.com/api/csa/api/v1/teams/users/me?isPrefetch=false&enableMembershipSummary=true";
            let v = client.json("csa", reqwest::Method::GET, url, None).await?;
            let chat = v["chats"]
                .as_array()
                .and_then(|a| a.iter().find(|c| c["id"] == id.as_str()))
                .cloned();
            let mut c = chat.unwrap_or_default();
            if let Some(o) = c.as_object_mut() {
                o.remove("lastMessage");
            }
            println!("{}", serde_json::to_string_pretty(&c)?);
        }
        Some("img") => {
            let url = args.get(1).context("usage: img <ams url>")?;
            let (client, _) = connect(&paths).await?;
            match client.download_image(url).await {
                Ok(b) => println!("ok {} bytes", b.len()),
                Err(e) => println!("error: {e}"),
            }
        }
        Some("send") => {
            let (Some(conv), Some(text)) = (args.get(1), args.get(2)) else {
                bail!("usage: send <conversation id> <text>")
            };
            let (client, cfg) = connect(&paths).await?;
            let name = cfg.display_name.clone().unwrap_or_default();
            let cmid = client
                .send_message(conv, &render::text_to_teams_html(text), &name, None)
                .await?;
            println!("sent (clientmessageid {cmid})");
        }
        Some("reply") => {
            let (Some(conv), Some(msg), Some(text)) = (args.get(1), args.get(2), args.get(3))
            else {
                bail!("usage: reply <conv> <msg id> <text>")
            };
            let (client, cfg) = connect(&paths).await?;
            let page = client.messages(conv, None, 50).await?;
            let orig = page
                .messages
                .iter()
                .find(|m| &m.id == msg)
                .context("message not in the last 50")?;
            let preview = render::plain_text(&render::parse_teams_html(&orig.content));
            let html = render::reply_html(
                &orig.id,
                &orig.sender_mri,
                &orig.sender_name,
                orig.compose_ms,
                &preview,
                text,
            );
            let qtd = teams_proto::chatsvc::quoted_messages_property(
                &orig.id,
                &orig.sender_mri,
                orig.compose_ms,
            );
            client
                .send_quote_reply(conv, &html, &cfg.display_name.unwrap_or_default(), &qtd)
                .await?;
            println!("replied");
        }
        Some("edit") => {
            let (Some(conv), Some(msg), Some(text)) = (args.get(1), args.get(2), args.get(3))
            else {
                bail!("usage: edit <conv> <msg id> <text>")
            };
            let (client, cfg) = connect(&paths).await?;
            client
                .edit_message(
                    conv,
                    msg,
                    &render::text_to_teams_html(text),
                    &cfg.display_name.unwrap_or_default(),
                )
                .await?;
            println!("edited");
        }
        Some("forward") => {
            let (Some(src), Some(msg), Some(target)) = (args.get(1), args.get(2), args.get(3))
            else {
                bail!("usage: forward <source conv> <message id> <target conv> [note]")
            };
            let (client, cfg) = connect(&paths).await?;
            let note = args.get(4).map(|t| render::text_to_teams_html(t));
            let r = client
                .forward(
                    src,
                    std::slice::from_ref(msg),
                    std::slice::from_ref(target),
                    note.as_deref(),
                    &cfg.display_name.unwrap_or_default(),
                )
                .await?;
            for f in r {
                println!(
                    "forwarded to {}: {} {:?}",
                    f.thread_id, f.status, f.message_id
                );
            }
        }
        Some(
            cmd @ ("save" | "unsave" | "pin" | "unpin" | "unread" | "clear-unread" | "mute"
            | "unmute" | "hide" | "unhide"),
        ) => {
            let conv = args.get(1).context("usage: <action> <conv> [message id]")?;
            let msg = args.get(2).cloned().unwrap_or_default();
            let (client, _) = connect(&paths).await?;
            match cmd {
                "save" | "unsave" => client.set_saved(conv, &msg, &msg, cmd == "save").await?,
                "pin" | "unpin" => client.set_pinned(conv, &msg, cmd == "pin").await?,
                "unread" => client.mark_unread(conv, &msg, "").await?,
                "clear-unread" => client.clear_unread_mark(conv).await?,
                "mute" | "unmute" => client.set_muted(conv, cmd == "mute").await?,
                _ => client.set_hidden(conv, cmd == "hide").await?,
            }
            println!("{cmd}: ok");
        }
        Some("saved") => {
            let (client, _) = connect(&paths).await?;
            for s in client.saved_messages().await? {
                println!(
                    "{} {} {}: {}",
                    s.conversation_id, s.message_id, s.sender_name, s.preview
                );
            }
        }
        Some("pins") => {
            let conv = args.get(1).context("usage: pins <chat id>")?;
            let (client, _) = connect(&paths).await?;
            for p in client.pinned(conv).await? {
                println!(
                    "{} folder={:?} at {}",
                    p.message_id, p.folder_id, p.pinned_at
                );
            }
        }
        Some("delete") => {
            let (Some(conv), Some(msg)) = (args.get(1), args.get(2)) else {
                bail!("usage: delete <conv> <msg id>")
            };
            let (client, _) = connect(&paths).await?;
            client.delete_message(conv, msg).await?;
            println!("deleted");
        }
        Some("unreact") => {
            let (Some(conv), Some(msg), Some(key)) = (args.get(1), args.get(2), args.get(3)) else {
                bail!("usage: unreact <conv> <msg id> <key>")
            };
            let (client, _) = connect(&paths).await?;
            client.remove_reaction(conv, msg, key).await?;
            println!("unreacted");
        }
        Some("send-image") => {
            let (Some(conv), Some(file)) = (args.get(1), args.get(2)) else {
                bail!("usage: send-image <conv> <file>")
            };
            let (client, cfg) = connect(&paths).await?;
            let data = std::fs::read(file)?;
            let name = std::path::Path::new(file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image.png")
                .to_string();
            let id = client.upload_image(conv, &name, data).await?;
            client
                .send_message(
                    conv,
                    &teams_proto::ams::image_html(&id, 320, 200),
                    &cfg.display_name.unwrap_or_default(),
                    None,
                )
                .await?;
            println!("image sent ({id})");
        }
        Some("post") => {
            let (Some(channel), Some(subject), Some(text)) =
                (args.get(1), args.get(2), args.get(3))
            else {
                bail!("usage: post <channel id> <subject> <text>")
            };
            let (client, cfg) = connect(&paths).await?;
            let cmid = client
                .send_message(
                    channel,
                    &render::text_to_teams_html(text),
                    &cfg.display_name.unwrap_or_default(),
                    Some(subject),
                )
                .await?;
            println!("posted (clientmessageid {cmid})");
        }
        Some("send-file") => {
            let (Some(conv), Some(file)) = (args.get(1), args.get(2)) else {
                bail!("usage: send-file <conv> <file>")
            };
            let (client, cfg) = connect(&paths).await?;
            let data = std::fs::read(file)?;
            let name = std::path::Path::new(file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string();
            let f = client
                .send_file(conv, &name, data, &cfg.display_name.unwrap_or_default())
                .await?;
            println!("file sent ({}, drive item {})", f.name, f.drive_item_id);
        }
        Some("react") => {
            let (Some(conv), Some(msg), Some(key)) = (args.get(1), args.get(2), args.get(3)) else {
                bail!("usage: react <conversation id> <message id> <key>")
            };
            let (client, _) = connect(&paths).await?;
            client.add_reaction(conv, msg, key).await?;
            println!("reacted");
        }
        Some("people") => {
            let q = args.get(1).context("usage: people <query>")?;
            let (client, cfg) = connect(&paths).await?;
            let me = self_mri(&client, &cfg).await?;
            let found = client.search_people(&me, q).await?;
            println!("{} result(s)", found.len());
            for p in &found {
                println!(
                    "{}  {}  {}  {}",
                    p.mri,
                    p.display_name,
                    p.email.as_deref().unwrap_or("-"),
                    p.job_title.as_deref().unwrap_or("-")
                );
            }
        }
        Some("presence") => {
            let (client, cfg) = connect(&paths).await?;
            let mut mris: Vec<String> = args[1..].to_vec();
            if mris.is_empty() {
                mris.push(self_mri(&client, &cfg).await?);
            }
            let found = client.get_presence(&mris).await?;
            println!("{} presence record(s)", found.len());
            for p in &found {
                println!(
                    "{} {} / {}{}{}",
                    p.mri,
                    p.availability,
                    p.activity,
                    if p.ooo { " ooo" } else { "" },
                    p.note
                        .as_ref()
                        .map(|n| format!(" note={:?}", n.text))
                        .unwrap_or_default()
                );
            }
        }
        Some("card") => {
            let (client, cfg) = connect(&paths).await?;
            let mri = match args.get(1) {
                Some(m) => m.clone(),
                None => self_mri(&client, &cfg).await?,
            };
            let c = client.profile_card(&mri).await?;
            println!("{c:#?}");
        }
        Some("horizons") => {
            let conv = args.get(1).context("usage: horizons <conv>")?;
            let (client, _) = connect(&paths).await?;
            let hs = client.consumption_horizons(conv).await?;
            println!("{} horizon(s)", hs.len());
            for h in &hs {
                println!("{} read up to {} at {}", h.mri, h.last_read_id, h.read_ms);
            }
            println!(
                "read receipts enabled: {}",
                client.read_receipts_enabled().await?
            );
        }
        Some("team") => {
            let (client, cfg) = connect(&paths).await?;
            let r = team::run(&client, &args[1..]).await;
            // A busy keyring must not fail (and so invite a retry of) a write that already ran.
            if let Err(e) = session::persist(&cfg, &client).await {
                eprintln!("warning: could not store the refreshed session: {e}");
            }
            r?;
        }
        Some("store") => {
            let (client, cfg) = connect(&paths).await?;
            let r = store::run(&client, &args[1..]).await;
            session::persist(&cfg, &client).await?;
            r?;
        }
        Some("drive") => {
            let (client, cfg) = connect(&paths).await?;
            let r = drive::run(&client, &args[1..]).await;
            session::persist(&cfg, &client).await?;
            r?;
        }
        Some("tasks" | "flows" | "onenote" | "lists") => {
            let (client, cfg) = connect(&paths).await?;
            let rest = &args[1..];
            let r = match args[0].as_str() {
                "tasks" => apps::tasks(&client, rest).await,
                "flows" => apps::flows(&client, rest).await,
                "onenote" => apps::onenote(&client, rest).await,
                _ => apps::lists(&client, rest).await,
            };
            session::persist(&cfg, &client).await?;
            r?;
        }
        Some("audiences") => {
            // Mints a token for every audience through the auth broker. Prints no token values.
            let (client, cfg) = connect(&paths).await?;
            for aud in teams_proto::broker::Audience::FIXED {
                match client.token_for(&aud).await {
                    Ok(_) => println!("ok    {}", aud.resource()),
                    Err(e) => println!("fail  {}: {e}", aud.resource()),
                }
            }
            // SharePoint is per host: use the host of the user's own OneDrive.
            let drive = client
                .json_as(
                    "drive",
                    &teams_proto::broker::Audience::Graph,
                    reqwest::Method::GET,
                    "https://graph.microsoft.com/v1.0/me/drive?$select=webUrl",
                    None,
                )
                .await?;
            let host = drive["webUrl"]
                .as_str()
                .and_then(|u| url::Url::parse(u).ok())
                .and_then(|u| u.host_str().map(str::to_owned))
                .unwrap_or_default();
            match teams_proto::broker::Audience::sharepoint(&host) {
                Some(sp) => match client.token_for(&sp).await {
                    Ok(_) => println!("ok    SharePoint (OneDrive host)"),
                    Err(e) => println!("fail  SharePoint (OneDrive host): {e}"),
                },
                None => println!("skip  SharePoint: OneDrive host not found"),
            }
            session::persist(&cfg, &client).await?;
        }
        Some("calendar") if args.get(1).is_some_and(|a| a.parse::<u32>().is_err()) => {
            let (client, cfg) = connect(&paths).await?;
            calendar_check(&client, &args[1..]).await?;
            session::persist(&cfg, &client).await?;
        }
        Some("calendar") => {
            // Read-only: prints only counts (per UTC day and per show-as), never event details.
            let days: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(7);
            let (client, cfg) = connect(&paths).await?;
            let today = calendar::Date::from_ms(teams_proto::time::now_ms());
            let (start, end) =
                calendar::week_range(today, calendar::View::Day(days), calendar::Weekday::Monday);
            let events = calendar::Calendar::view(&client, start, end).await?;
            let mut per_day = std::collections::BTreeMap::new();
            let mut per_show_as = std::collections::BTreeMap::new();
            for e in &events {
                *per_day
                    .entry(calendar::Date::from_ms(e.start_ms))
                    .or_insert(0usize) += 1;
                *per_show_as
                    .entry(format!("{:?}", e.show_as))
                    .or_insert(0usize) += 1;
            }
            println!("{} events from {today} for {days} day(s)", events.len());
            for (day, n) in &per_day {
                println!("  {day}  {n}");
            }
            for (show_as, n) in &per_show_as {
                println!("  {show_as:<18} {n}");
            }
            session::persist(&cfg, &client).await?;
        }
        Some("tail") => {
            let (client, _) = connect(&paths).await?;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(teams_proto::trouter::run(client, tx));
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Some(TrouterEvent::Connected) => println!("-- connected"),
                        Some(TrouterEvent::Resync) => println!("-- resync"),
                        Some(TrouterEvent::Disconnected(r)) => println!("-- disconnected: {r}"),
                        Some(TrouterEvent::Event(ev)) => match *ev {
                            Event::NewMessage(m) if m.is_chat_content() => println!("[{}] {} in {}: {}", fmt_time(m.compose_ms), m.sender_name, m.conversation_id, preview(&m.content)),
                            Event::MessageUpdate(m) => println!("~ update {} in {} (reactions {})", m.id, m.conversation_id, m.reactions.len()),
                            Event::Typing { conversation_id, from } => println!("… {from} typing in {conversation_id}"),
                            other => println!("{other:?}"),
                        },
                        None => break,
                    },
                    _ = tokio::signal::ctrl_c() => break,
                }
            }
        }
        _ => {
            println!(
                "usage: kelma-cli <command>\n\
                   login\n\
                   logout\n\
                   diag [--log FILE] [--out PATH]\n\
                   chats [n]\n\
                   teams\n\
                   calendar [days]\n\
                   calendar calendars|rooms|mailbox|schedule|worklocation|detail|ics\n\
                   activity [n]\n\
                   read <conv> [n]\n\
                   send <conv> <text>\n\
                   send-image <conv> <file>\n\
                   send-file <conv> <file>\n\
                   post <channel> <subject> <text>\n\
                   react <conv> <msg> <key>\n\
                   forward <src> <msg> <target> [note]\n\
                   save/unsave/pin/unpin/unread <conv> <msg>\n\
                   mute/unmute/hide/unhide <conv>\n\
                   saved\n\
                   pins <chat>\n\
                   audiences\n\
                   people <q>\n\
                   presence [mri..]\n\
                   card [mri]\n\
                   horizons <conv>\n\
                   drive <ls|search|recent|shared|favorites|recordings|libraries|sync-help|bin|mkdir|rm|selftest|purge>\n\
                   tasks [todo <n>|plan <n>]\n\
                   flows [runs <n> [top]]\n\
                   onenote [<notebook> [<section> [<page>]]]\n\
                   lists [<site> [<list>]]\n\
                   team <ls|discover|join|join-code|code|new|rm|edit|settings|archive|show|hide|channel|tag|members|community|storyline|sandbox|probe|raw>\n\
                   store <bar|more|home|search|app|mine|org|settings|install|uninstall|update|pin|unpin|move|probe>\n\
                   tail"
            );
        }
    }
    Ok(())
}
