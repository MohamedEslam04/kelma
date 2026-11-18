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
//! kelma-cli audiences             mint a token for every audience (prints no tokens)
//! kelma-cli tail                  print live events until Ctrl+C
//! kelma-cli logout                remove the stored refresh token
//! ```

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

async fn connect(paths: &Paths) -> Result<(Client, session::Config)> {
    let cfg = paths.load_config()?;
    let sess = session::stored_session(&cfg)?.context("not signed in: run `kelma-cli login`")?;
    let client = Client::resume(sess).await?;
    session::persist(&cfg, &client).await?;
    Ok((client, cfg))
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
            client
                .send_message(conv, &html, &cfg.display_name.unwrap_or_default(), None)
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
                   chats [n]\n\
                   teams\n\
                   calendar [days]\n\
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
                   tail"
            );
        }
    }
    Ok(())
}
