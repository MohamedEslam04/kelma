//! `kelma-cli`: exercise the protocol from a terminal.
//!
//! ```text
//! kelma-cli login                 sign in with a device code
//! kelma-cli chats [n]             list recent conversations
//! kelma-cli teams                 list teams and channels
//! kelma-cli read <conv> [n]       print the last n messages
//! kelma-cli send <conv> <text>    send a message (use 48:notes for your own notes)
//! kelma-cli react <conv> <msg> <key>
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
        Some("react") => {
            let (Some(conv), Some(msg), Some(key)) = (args.get(1), args.get(2), args.get(3)) else {
                bail!("usage: react <conversation id> <message id> <key>")
            };
            let (client, _) = connect(&paths).await?;
            client.add_reaction(conv, msg, key).await?;
            println!("reacted");
        }
        Some("tail") => {
            let (client, _) = connect(&paths).await?;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(teams_proto::trouter::run(client, tx));
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Some(TrouterEvent::Connected) => println!("-- connected"),
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
                "usage: kelma-cli <login|logout|chats [n]|teams|read <conv> [n]|send <conv> <text>|react <conv> <msg> <key>|tail>"
            );
        }
    }
    Ok(())
}
