//! `kelma-cli find …`: live checks of search, quick views and shared files (plan chat 7.x).
//! Prints counts and key shapes only, never names or message content.

use anyhow::{Result, bail};
use serde_json::Value;
use teams_proto::Client;
use teams_proto::search::{SearchFilters, mentions_query};

/// Key structure of a JSON value: types and lengths. Never prints values.
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
        Value::String(s) => println!("{pad}{key}: string({})", s.len()),
        Value::Number(n) => println!("{pad}{key}: {n}"),
        Value::Bool(b) => println!("{pad}{key}: {b}"),
        Value::Null => println!("{pad}{key}: null"),
    }
}

fn feed_url(chat_service: &str, conv: &str, n: &str) -> String {
    format!(
        "{chat_service}/v1/users/ME/conversations/{}/messages?view=msnp24Equivalent|supportsMessageProperties&pageSize={n}&startTime=1",
        teams_proto::chatsvc::encode_conversation(conv)
    )
}

pub async fn run(client: &Client, cfg: &session::Config, args: &[String]) -> Result<()> {
    let me = match &cfg.self_mri {
        Some(m) => m.clone(),
        None => client.self_mri().await?,
    };
    match args.first().map(String::as_str) {
        // `find search <query> [in <conv>]`: message search; counts and id checks only.
        Some("search") => {
            let q = args.get(1).map(String::as_str).unwrap_or("meeting");
            let filters = SearchFilters {
                in_thread: args.get(3).cloned(),
                ..Default::default()
            };
            let page = client.search_messages(&me, q, &filters, 0, 10).await?;
            println!(
                "total {}, {} hits, more {}",
                page.total,
                page.hits.len(),
                page.more
            );
            let (mut found, mut found_near, mut missing, mut replies) = (0, 0, 0, 0);
            for h in &page.hits {
                // A channel reply lives in its thread.
                let target = if h.root_id.is_empty() {
                    h.conversation_id.clone()
                } else {
                    format!("{};messageid={}", h.conversation_id, h.root_id)
                };
                let before = h.root_id.is_empty().then_some(h.time_ms + 60_000);
                let msgs = client.messages(&target, before, 30).await;
                let ids: Vec<String> = msgs
                    .map(|p| p.messages.into_iter().map(|m| m.id).collect())
                    .unwrap_or_default();
                let reply = !h.root_id.is_empty();
                if reply {
                    replies += 1;
                }
                if ids.contains(&h.message_id) {
                    found += 1;
                } else if reply {
                    // A reply's id is only known to about 10 ms: look for the nearest one.
                    let near = h.message_id.parse::<i64>().ok().is_some_and(|t| {
                        ids.iter()
                            .filter_map(|i| i.parse::<i64>().ok())
                            .any(|i| (i - t).abs() < 1500)
                    });
                    if near {
                        found_near += 1;
                    } else {
                        missing += 1;
                    }
                } else {
                    missing += 1;
                }
            }
            println!(
                "ids: {found} found by message id, {found_near} reply found near its time, {missing} not found; {replies} replies"
            );
        }
        // `find follow-raw <PUT|DELETE> <value|-> <root id>`: writes the `follow` emotion on a
        // post of the sandbox channel only (KELMA_SANDBOX_FILE).
        Some("follow-raw") => {
            let sb: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var(
                "KELMA_SANDBOX_FILE",
            )?)?)?;
            let chan = sb["channels"]["tests"].as_str().unwrap_or_default();
            let method = args.get(1).map(String::as_str).unwrap_or("PUT");
            let value = args.get(2).map(String::as_str).unwrap_or("0");
            let root = args.get(3).map(String::as_str).unwrap_or_default();
            let url = format!(
                "{}/v1/users/ME/conversations/{}/messages/{root}/properties?name=emotions",
                client.chat_service().await,
                teams_proto::chatsvc::encode_conversation(chan)
            );
            let body = if value == "-" {
                serde_json::json!({"emotions": {"key": "follow"}})
            } else {
                serde_json::json!({"emotions": {"key": "follow", "value": value}})
            };
            let m = if method == "DELETE" {
                reqwest::Method::DELETE
            } else {
                reqwest::Method::PUT
            };
            let (status, text) = client.request(m, &url, Some(&body), &[]).await?;
            println!(
                "{method} follow value={value}: status {status} ({} bytes)",
                text.len()
            );
        }
        // `find props <root id>`: emotions of a sandbox channel post (keys, values, user counts).
        Some("props") => {
            let sb: Value = serde_json::from_str(&std::fs::read_to_string(std::env::var(
                "KELMA_SANDBOX_FILE",
            )?)?)?;
            let chan = sb["channels"]["tests"].as_str().unwrap_or_default();
            let root = args.get(1).map(String::as_str).unwrap_or_default();
            let url = feed_url(&client.chat_service().await, chan, "10");
            let v = client
                .json("props", reqwest::Method::GET, &url, None)
                .await?;
            for m in v["messages"].as_array().into_iter().flatten() {
                if m["id"].as_str() != Some(root) {
                    continue;
                }
                let p = &m["properties"];
                println!(
                    "property keys: {:?}",
                    p.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())
                );
                let e = match &p["emotions"] {
                    Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
                    other => other.clone(),
                };
                for x in e.as_array().into_iter().flatten() {
                    println!(
                        "emotion key={} users={} first value={}",
                        x["key"],
                        x["users"].as_array().map_or(0, Vec::len),
                        x["users"][0]["value"]
                    );
                }
            }
        }
        // `find followed`: the followed threads (counts only).
        Some("followed") => {
            let list = client.followed_threads(100).await?;
            println!(
                "followed threads: {} ({} implicit, {} explicit)",
                list.len(),
                list.iter().filter(|t| t.implicit).count(),
                list.iter().filter(|t| !t.implicit).count()
            );
        }
        // `find shared <conv>`: files and links shared in a chat or channel (counts only).
        Some("shared") => {
            let conv = args.get(1).map(String::as_str).unwrap_or("48:notes");
            let items = client.shared_items(&me, conv).await?;
            let files = items
                .iter()
                .filter(|i| i.kind == teams_proto::views::SharedKind::File)
                .count();
            println!(
                "shared in the conversation: {} files, {} links, {} with a sender, {} with a date",
                files,
                items.len() - files,
                items.iter().filter(|i| !i.shared_by.is_empty()).count(),
                items.iter().filter(|i| i.shared_ms > 0).count()
            );
        }
        // `find folder <group id> <channel id>`: the files in a channel's folder (count only).
        Some("folder") => {
            let (Some(group), Some(channel)) = (args.get(1), args.get(2)) else {
                bail!("usage: find folder <group id> <channel id>")
            };
            let f = client.channel_files_folder(group, channel).await?;
            let (drive, folder) = (f.drive_id, f.item_id);
            let page = onedrive::children(client, &drive, &folder, None).await?;
            println!(
                "channel folder: {} items, {} folders",
                page.items.len(),
                page.items.iter().filter(|i| i.is_folder).count()
            );
        }
        // `find mentions`: how many hits really mention me.
        Some("mentions") => {
            let q = mentions_query(&me).unwrap_or_default();
            let page = client
                .search_messages(&me, &q, &SearchFilters::default(), 0, 50)
                .await?;
            let real = page.hits.iter().filter(|h| h.is_mention).count();
            println!(
                "mentions: total {}, first page {} hits, {} flagged IsMentioned",
                page.total,
                page.hits.len(),
                real
            );
        }
        // `find feed <system conversation> [n]`: key structure of a system conversation.
        Some("feed") => {
            let conv = args.get(1).map(String::as_str).unwrap_or("48:threads");
            let n = args.get(2).map(String::as_str).unwrap_or("5");
            let url = feed_url(&client.chat_service().await, conv, n);
            let v = client
                .json("feed", reqwest::Method::GET, &url, None)
                .await?;
            shape(&v, 0, "root");
            // Enum-like fields of activity rows (no names or content).
            for m in v["messages"].as_array().into_iter().flatten() {
                let a = &m["properties"]["activity"];
                println!(
                    "id={} edit={} chain={} type={} reason={} subtype={} threadType={} followType={} private={}",
                    m["id"],
                    m["properties"]["edittime"],
                    a["sourceReplyChainId"],
                    a["activityType"],
                    a["activityReason"],
                    a["activitySubtype"],
                    a["sourceThreadType"],
                    a["activityContext"]["followType"],
                    a["activityContext"]["sourceThreadIsPrivateChannel"],
                );
            }
        }
        _ => bail!(
            "usage: find <search q [in conv]|mentions|followed|shared conv|folder group channel|feed conv [n]>"
        ),
    }
    Ok(())
}
