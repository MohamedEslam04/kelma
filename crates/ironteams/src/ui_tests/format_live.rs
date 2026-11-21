//! Live check of the composer formatting (plan chat 5.4) against the real account, only in
//! 48:notes, and the messages are deleted again:
//! `cargo test -p ironteams live_format -- --ignored --nocapture`. Sends one message per construct
//! and reads each back: what the server stored must decode to what was sent.

use super::*;

#[tokio::test]
#[ignore = "live: posts to 48:notes and deletes the messages afterwards"]
async fn live_format_constructs_round_trip_through_the_server() {
    let paths = session::Paths::new().unwrap();
    let cfg = paths.load_config().unwrap();
    let sess = session::stored_session(&cfg).unwrap().expect("signed in");
    let client = Client::resume(sess).await.unwrap();
    session::persist(&cfg, &client).await.unwrap();
    let me = cfg.self_mri.clone().expect("self mri in config");
    init_platform();
    let window = AppWindow::new().unwrap();
    let ui = UiHandle::new(window.as_weak());
    let e = Engine::new(
        client.clone(),
        Store::open_in_memory().unwrap(),
        cfg,
        ui,
        me,
        "ironteams live test".into(),
    );
    e.clone().handle(Cmd::SelectChat(NOTES.into())).await;

    let sources = [
        "**b** *i* ++u++ ~~s~~ `c`",
        "{color:#ff0000}red{/} {bg:#fdd472}hl{/} {size:large}big{/}",
        "# H1\n\n## H2\n\n### H3",
        "> quoted line",
        "- a\n  - b\n- c",
        "1. one\n2. two",
        "```rust\nlet x = 1;\n```",
        "| a | b |\n| --- | --- |\n| 1 | 2 |",
        "see [the docs](https://example.com)",
    ];
    let stamp = teams_proto::time::now_ms();
    let mut sent = Vec::new();
    for (i, src) in sources.iter().enumerate() {
        let text = format!("ironteams format live {stamp} {i}\n\n{src}");
        e.clone()
            .handle(Cmd::Send(text.clone(), String::new()))
            .await;
        let tag = format!("ironteams format live {stamp} {i}");
        sent.push((tag, text));
    }
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let page = client.messages(NOTES, None, 40).await.unwrap();
    let mut bad = Vec::new();
    for (tag, text) in &sent {
        let Some(m) = page
            .messages
            .iter()
            .find(|m| m.content.contains(tag.as_str()))
        else {
            bad.push(format!("not on the server: {text:?}"));
            continue;
        };
        let got = render::parse_rich_html(&m.content);
        let want = render::markdown_to_rich(text);
        println!("{text:?}\n  server html: {}", m.content);
        if got != want {
            bad.push(format!("{text:?}\n  got  {got:?}\n  want {want:?}"));
        }
    }
    for (tag, _) in &sent {
        if let Some(m) = page
            .messages
            .iter()
            .find(|m| m.content.contains(tag.as_str()))
        {
            let _ = client.delete_message(NOTES, &m.id).await;
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// `cargo test -p ironteams live_important -- --ignored --nocapture`: "Mark as important" through the
/// real send path and outbox into 48:notes; the server must read it back as `high`. The next
/// message (no flag) must be standard. Both are deleted again.
#[tokio::test]
#[ignore = "live: posts to 48:notes and deletes the messages afterwards"]
async fn live_important_message_reads_back_as_high() {
    let paths = session::Paths::new().unwrap();
    let cfg = paths.load_config().unwrap();
    let sess = session::stored_session(&cfg).unwrap().expect("signed in");
    let client = Client::resume(sess).await.unwrap();
    session::persist(&cfg, &client).await.unwrap();
    let me = cfg.self_mri.clone().expect("self mri in config");
    init_platform();
    let window = AppWindow::new().unwrap();
    let ui = UiHandle::new(window.as_weak());
    let e = Engine::new(
        client.clone(),
        Store::open_in_memory().unwrap(),
        cfg,
        ui,
        me,
        "ironteams live test".into(),
    );
    e.clone().handle(Cmd::SelectChat(NOTES.into())).await;

    let stamp = teams_proto::time::now_ms();
    let important = format!("ironteams important live {stamp} a");
    let standard = format!("ironteams important live {stamp} b");
    e.clone().handle(Cmd::SetImportant(true)).await;
    e.clone()
        .handle(Cmd::Send(important.clone(), String::new()))
        .await;
    e.clone()
        .handle(Cmd::Send(standard.clone(), String::new()))
        .await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let page = client.messages(NOTES, None, 40).await.unwrap();
    let find = |t: &str| page.messages.iter().find(|m| m.content.contains(t));
    let (a, b) = (find(&important), find(&standard));
    println!(
        "important -> {:?}, standard -> {:?}",
        a.map(|m| m.importance),
        b.map(|m| m.importance)
    );
    for m in [a, b].into_iter().flatten() {
        let _ = client.delete_message(NOTES, &m.id).await;
    }
    use teams_proto::delivery::Importance;
    assert_eq!(
        a.expect("important message").importance,
        Importance::Important
    );
    assert_eq!(
        b.expect("standard message").importance,
        Importance::Standard
    );
}
