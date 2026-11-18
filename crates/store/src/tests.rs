use super::*;

fn conv(id: &str, kind: ConvKind, title: &str, activity: i64) -> ConversationRow {
    ConversationRow {
        id: id.into(),
        kind,
        title: title.into(),
        team_id: None,
        team_name: None,
        last_activity_ms: activity,
        last_preview: String::new(),
        unread: false,
        favorite: false,
        read_horizon_ms: 0,
    }
}

fn msg(id: &str, conv: &str, ms: i64) -> MessageRow {
    MessageRow {
        id: id.into(),
        conversation_id: conv.into(),
        parent_id: None,
        sender_mri: "8:orgid:a".into(),
        sender_name: "A".into(),
        compose_ms: ms,
        edited_ms: None,
        deleted: false,
        message_type: "RichText/Html".into(),
        content_html: format!("<p>{id}</p>"),
        client_message_id: None,
        reactions_json: "[]".into(),
    }
}

fn user(mri: &str, name: &str, avatar: Option<Vec<u8>>) -> UserRow {
    UserRow {
        mri: mri.into(),
        display_name: name.into(),
        email: None,
        avatar,
        updated_ms: 1,
    }
}

fn ids(v: &[ConversationRow]) -> Vec<&str> {
    v.iter().map(|c| c.id.as_str()).collect()
}

fn msg_ids(v: &[MessageRow]) -> Vec<&str> {
    v.iter().map(|m| m.id.as_str()).collect()
}

#[test]
fn conversation_roundtrip_all_fields() {
    let s = Store::open_in_memory().unwrap();
    let c = ConversationRow {
        id: "19:x".into(),
        kind: ConvKind::Channel,
        title: "General".into(),
        team_id: Some("t1".into()),
        team_name: Some("Team".into()),
        last_activity_ms: 42,
        last_preview: "hi".into(),
        unread: true,
        favorite: true,
        read_horizon_ms: 7,
    };
    s.upsert_conversation(&c).unwrap();
    assert_eq!(s.conversation("19:x").unwrap(), Some(c));
    assert_eq!(s.conversation("nope").unwrap(), None);
}

#[test]
fn every_kind_roundtrips() {
    let s = Store::open_in_memory().unwrap();
    for (i, k) in [
        ConvKind::OneOnOne,
        ConvKind::Group,
        ConvKind::Channel,
        ConvKind::Meeting,
        ConvKind::Notes,
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("c{i}");
        s.upsert_conversation(&conv(&id, k, "t", 1)).unwrap();
        assert_eq!(s.conversation(&id).unwrap().unwrap().kind, k);
    }
}

#[test]
fn upsert_conversation_updates_but_preserves_favorite() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_conversation(&conv("c", ConvKind::Group, "old", 1))
        .unwrap();
    s.set_favorite("c", true).unwrap();

    let mut update = conv("c", ConvKind::Group, "new", 5);
    update.favorite = false;
    update.unread = true;
    update.last_preview = "p".into();
    s.upsert_conversation(&update).unwrap();

    let got = s.conversation("c").unwrap().unwrap();
    assert!(got.favorite, "favorite must be preserved");
    assert_eq!(got.title, "new");
    assert_eq!(got.last_activity_ms, 5);
    assert!(got.unread);
    assert_eq!(got.last_preview, "p");

    s.set_favorite("c", false).unwrap();
    assert!(!s.conversation("c").unwrap().unwrap().favorite);
}

#[test]
fn new_conversation_honours_favorite_on_insert() {
    let s = Store::open_in_memory().unwrap();
    let mut c = conv("c", ConvKind::Group, "t", 1);
    c.favorite = true;
    s.upsert_conversation(&c).unwrap();
    assert!(s.conversation("c").unwrap().unwrap().favorite);
}

#[test]
fn conversations_ordered_by_activity_desc_and_limited() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_conversation(&conv("a", ConvKind::Group, "a", 10))
        .unwrap();
    s.upsert_conversation(&conv("b", ConvKind::Group, "b", 30))
        .unwrap();
    s.upsert_conversation(&conv("c", ConvKind::Group, "c", 20))
        .unwrap();
    let all = s.conversations(ConvFilter::All, 10).unwrap();
    assert_eq!(ids(&all), ["b", "c", "a"]);
    let two = s.conversations(ConvFilter::All, 2).unwrap();
    assert_eq!(ids(&two), ["b", "c"]);
    assert!(s.conversations(ConvFilter::All, 0).unwrap().is_empty());
}

#[test]
fn conversation_filters() {
    let s = Store::open_in_memory().unwrap();
    let mut one = conv("one", ConvKind::OneOnOne, "1", 60);
    one.unread = true;
    let group = conv("grp", ConvKind::Group, "g", 50);
    let mut chan = conv("chan", ConvKind::Channel, "c", 40);
    chan.team_id = Some("t".into());
    let meet = conv("meet", ConvKind::Meeting, "m", 30);
    let notes = conv("notes", ConvKind::Notes, "n", 20);
    for c in [&one, &group, &chan, &meet, &notes] {
        s.upsert_conversation(c).unwrap();
    }
    s.set_favorite("chan", true).unwrap();
    s.set_favorite("meet", true).unwrap();

    let f = |f| ids_owned(&s.conversations(f, 100).unwrap());
    assert_eq!(f(ConvFilter::All).len(), 5);
    assert_eq!(f(ConvFilter::Unread), ["one"]);
    assert_eq!(f(ConvFilter::Chats), ["one", "grp", "meet", "notes"]);
    assert_eq!(f(ConvFilter::Channels), ["chan"]);
    assert_eq!(f(ConvFilter::Favorites), ["chan", "meet"]);
}

fn ids_owned(v: &[ConversationRow]) -> Vec<String> {
    v.iter().map(|c| c.id.clone()).collect()
}

#[test]
fn mark_read_is_monotonic_and_clears_unread() {
    let s = Store::open_in_memory().unwrap();
    let mut c = conv("c", ConvKind::Group, "t", 1);
    c.unread = true;
    s.upsert_conversation(&c).unwrap();

    s.mark_read("c", 100).unwrap();
    let got = s.conversation("c").unwrap().unwrap();
    assert!(!got.unread);
    assert_eq!(got.read_horizon_ms, 100);

    s.mark_read("c", 50).unwrap();
    assert_eq!(s.conversation("c").unwrap().unwrap().read_horizon_ms, 100);
    s.mark_read("c", 150).unwrap();
    assert_eq!(s.conversation("c").unwrap().unwrap().read_horizon_ms, 150);

    // Unknown conversation is a no-op.
    s.mark_read("missing", 1).unwrap();
}

#[test]
fn mark_unread_lowers_the_horizon_and_survives_until_read() {
    let s = Store::open_in_memory().unwrap();
    let mut c = conv("c", ConvKind::Group, "t", 1);
    c.read_horizon_ms = 500;
    s.upsert_conversation(&c).unwrap();
    assert_eq!(s.unread_mark("c").unwrap(), None);

    s.mark_unread("c", 300).unwrap();
    let got = s.conversation("c").unwrap().unwrap();
    assert!(got.unread);
    assert_eq!(
        got.read_horizon_ms, 300,
        "horizon moves back to before the message"
    );
    assert_eq!(s.unread_mark("c").unwrap(), Some(300));

    // Reading clears the mark.
    s.mark_read("c", 600).unwrap();
    assert_eq!(s.unread_mark("c").unwrap(), None);
    let got = s.conversation("c").unwrap().unwrap();
    assert!(!got.unread);
    assert_eq!(got.read_horizon_ms, 600);

    // Unknown conversation: no row to change, and no mark left behind.
    s.mark_unread("missing", 1).unwrap();
    assert_eq!(s.unread_mark("missing").unwrap(), None);
}

#[test]
fn touch_conversation_only_moves_forward() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_conversation(&conv("c", ConvKind::Group, "t", 100))
        .unwrap();

    s.touch_conversation("c", 200, "newer", true).unwrap();
    let got = s.conversation("c").unwrap().unwrap();
    assert_eq!(got.last_activity_ms, 200);
    assert_eq!(got.last_preview, "newer");
    assert!(got.unread);

    // Older: no change at all.
    s.touch_conversation("c", 150, "older", false).unwrap();
    let got = s.conversation("c").unwrap().unwrap();
    assert_eq!(got.last_activity_ms, 200);
    assert_eq!(got.last_preview, "newer");
    assert!(got.unread);

    // Equal: no change.
    s.touch_conversation("c", 200, "same", false).unwrap();
    assert_eq!(s.conversation("c").unwrap().unwrap().last_preview, "newer");

    // Newer with unread=false does not clear an existing unread flag.
    s.touch_conversation("c", 300, "mine", false).unwrap();
    let got = s.conversation("c").unwrap().unwrap();
    assert_eq!(got.last_preview, "mine");
    assert!(got.unread);

    s.mark_read("c", 300).unwrap();
    s.touch_conversation("c", 400, "x", false).unwrap();
    assert!(!s.conversation("c").unwrap().unwrap().unread);

    s.touch_conversation("missing", 1, "x", true).unwrap();
}

#[test]
fn search_conversations_arabic_and_case_insensitive() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_conversation(&conv("ar", ConvKind::Group, "مجموعة الإدارة", 10))
        .unwrap();
    s.upsert_conversation(&conv("en", ConvKind::Group, "Project Alpha", 20))
        .unwrap();
    s.upsert_conversation(&conv("pct", ConvKind::Group, "100% done_ok", 5))
        .unwrap();
    s.upsert_conversation(&conv("fr", ConvKind::Group, "Équipe", 1))
        .unwrap();

    assert_eq!(ids(&s.search_conversations("الإدارة", 10).unwrap()), ["ar"]);
    assert_eq!(ids(&s.search_conversations("مجموعة", 10).unwrap()), ["ar"]);
    assert_eq!(ids(&s.search_conversations("alpha", 10).unwrap()), ["en"]);
    assert_eq!(ids(&s.search_conversations("PROJECT", 10).unwrap()), ["en"]);
    assert_eq!(ids(&s.search_conversations("équipe", 10).unwrap()), ["fr"]);
    // LIKE wildcards are literal.
    assert_eq!(ids(&s.search_conversations("%", 10).unwrap()), ["pct"]);
    assert_eq!(ids(&s.search_conversations("_ok", 10).unwrap()), ["pct"]);
    assert!(s.search_conversations("zzz", 10).unwrap().is_empty());
    // Empty query matches everything, newest first, limited.
    assert_eq!(ids(&s.search_conversations("", 2).unwrap()), ["en", "ar"]);
}

#[test]
fn messages_before_pages_ascending_with_limit_and_before() {
    let s = Store::open_in_memory().unwrap();
    for i in 1..=10 {
        s.upsert_message(&msg(&format!("m{i}"), "c", i * 10))
            .unwrap();
    }
    s.upsert_message(&msg("other", "c2", 55)).unwrap();

    let newest = s.messages_before("c", None, 3).unwrap();
    assert_eq!(msg_ids(&newest), ["m8", "m9", "m10"]);

    let page2 = s.messages_before("c", Some(80), 3).unwrap();
    assert_eq!(msg_ids(&page2), ["m5", "m6", "m7"]);

    // `before` is exclusive.
    let page = s.messages_before("c", Some(30), 10).unwrap();
    assert_eq!(msg_ids(&page), ["m1", "m2"]);

    assert!(s.messages_before("c", Some(10), 5).unwrap().is_empty());
    assert!(s.messages_before("c", None, 0).unwrap().is_empty());
    assert_eq!(s.messages_before("c", None, 100).unwrap().len(), 10);
    assert!(s.messages_before("nope", None, 5).unwrap().is_empty());
}

#[test]
fn messages_with_equal_compose_ms_are_stable() {
    let s = Store::open_in_memory().unwrap();
    for id in ["3", "1", "2"] {
        s.upsert_message(&msg(id, "c", 100)).unwrap();
    }
    assert_eq!(
        msg_ids(&s.messages_before("c", None, 10).unwrap()),
        ["1", "2", "3"]
    );
}

#[test]
fn thread_replies_separate_from_top_level() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_message(&msg("root", "ch", 10)).unwrap();
    s.upsert_message(&msg("top2", "ch", 40)).unwrap();
    for (id, ms) in [("r3", 30), ("r1", 15), ("r2", 20)] {
        let mut m = msg(id, "ch", ms);
        m.parent_id = Some("root".into());
        s.upsert_message(&m).unwrap();
    }

    assert_eq!(
        msg_ids(&s.messages_before("ch", None, 10).unwrap()),
        ["root", "top2"]
    );
    assert_eq!(
        msg_ids(&s.thread_replies("root", 10).unwrap()),
        ["r1", "r2", "r3"]
    );
    assert_eq!(msg_ids(&s.thread_replies("root", 2).unwrap()), ["r1", "r2"]);
    assert_eq!(s.reply_count("root").unwrap(), 3);
    assert_eq!(s.reply_count("top2").unwrap(), 0);
    assert!(s.thread_replies("top2", 10).unwrap().is_empty());
    // Replies do not count towards the conversation's top-level cursor.
    assert_eq!(s.latest_compose_ms("ch").unwrap(), Some(40));
}

#[test]
fn latest_compose_ms() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.latest_compose_ms("c").unwrap(), None);
    s.upsert_message(&msg("a", "c", 5)).unwrap();
    s.upsert_message(&msg("b", "c", 9)).unwrap();
    s.upsert_message(&msg("z", "other", 99)).unwrap();
    assert_eq!(s.latest_compose_ms("c").unwrap(), Some(9));
}

#[test]
fn edits_update_in_place() {
    let s = Store::open_in_memory().unwrap();
    let mut m = msg("1", "c", 10);
    m.client_message_id = Some("cid".into());
    s.upsert_message(&m).unwrap();

    let mut edited = m.clone();
    edited.content_html = "<p>edited</p>".into();
    edited.edited_ms = Some(99);
    edited.reactions_json = r#"[{"key":"like"}]"#.into();
    // Immutable-on-update fields in the incoming row are ignored.
    edited.compose_ms = 12345;
    edited.sender_name = "Someone else".into();
    s.upsert_message(&edited).unwrap();

    let got = s.message("1").unwrap().unwrap();
    assert_eq!(got.content_html, "<p>edited</p>");
    assert_eq!(got.edited_ms, Some(99));
    assert_eq!(got.reactions_json, r#"[{"key":"like"}]"#);
    assert_eq!(got.compose_ms, 10);
    assert_eq!(got.sender_name, "A");
    assert_eq!(got.client_message_id.as_deref(), Some("cid"));
    // Still exactly one row.
    assert_eq!(s.messages_before("c", None, 10).unwrap().len(), 1);
    assert_eq!(s.message("missing").unwrap(), None);
}

#[test]
fn deletes_update_in_place() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_message(&msg("1", "c", 10)).unwrap();
    s.upsert_message(&msg("2", "c", 20)).unwrap();

    s.delete_message_local("1").unwrap();
    let got = s.message("1").unwrap().unwrap();
    assert!(got.deleted);
    assert_eq!(got.content_html, "");
    assert!(!s.message("2").unwrap().unwrap().deleted);
    assert_eq!(s.messages_before("c", None, 10).unwrap().len(), 2);

    // Server-side delete via upsert.
    let mut d = msg("2", "c", 20);
    d.deleted = true;
    d.content_html = String::new();
    s.upsert_message(&d).unwrap();
    assert!(s.message("2").unwrap().unwrap().deleted);

    s.delete_message_local("missing").unwrap();
}

#[test]
fn upsert_user_preserves_avatar_when_new_is_none() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_user(&user("8:a", "Ann", Some(vec![1, 2, 3])))
        .unwrap();

    let mut u = user("8:a", "Ann B", None);
    u.email = Some("a@x.com".into());
    u.updated_ms = 9;
    s.upsert_user(&u).unwrap();
    let got = s.user("8:a").unwrap().unwrap();
    assert_eq!(got.display_name, "Ann B");
    assert_eq!(got.email.as_deref(), Some("a@x.com"));
    assert_eq!(got.avatar, Some(vec![1, 2, 3]));
    assert_eq!(got.updated_ms, 9);

    s.upsert_user(&user("8:a", "Ann B", Some(vec![9]))).unwrap();
    assert_eq!(s.user("8:a").unwrap().unwrap().avatar, Some(vec![9]));
    assert_eq!(s.user("8:none").unwrap(), None);
}

#[test]
fn users_missing_profile() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_user(&user("8:have", "Have", None)).unwrap();
    s.upsert_user(&user("8:blank", "", None)).unwrap();
    let q: Vec<String> = ["8:x", "8:have", "8:blank", "8:y", "8:x"]
        .into_iter()
        .map(String::from)
        .collect();
    assert_eq!(
        s.users_missing_profile(&q).unwrap(),
        ["8:x", "8:blank", "8:y"]
    );
    assert!(s.users_missing_profile(&[]).unwrap().is_empty());
}

#[test]
fn teams_and_channels() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_team(&TeamRow {
        id: "t2".into(),
        name: "beta".into(),
    })
    .unwrap();
    s.upsert_team(&TeamRow {
        id: "t1".into(),
        name: "Alpha".into(),
    })
    .unwrap();
    s.upsert_team(&TeamRow {
        id: "t2".into(),
        name: "Beta".into(),
    })
    .unwrap();
    let teams = s.teams().unwrap();
    assert_eq!(
        teams,
        [
            TeamRow {
                id: "t1".into(),
                name: "Alpha".into()
            },
            TeamRow {
                id: "t2".into(),
                name: "Beta".into()
            }
        ]
    );

    let mk = |id: &str, title: &str, team: &str| {
        let mut c = conv(id, ConvKind::Channel, title, 1);
        c.team_id = Some(team.into());
        c
    };
    s.upsert_conversation(&mk("c1", "random", "t1")).unwrap();
    s.upsert_conversation(&mk("c2", "General", "t1")).unwrap();
    s.upsert_conversation(&mk("c3", "General", "t2")).unwrap();
    let mut chat = conv("chat", ConvKind::Group, "x", 1);
    chat.team_id = Some("t1".into());
    s.upsert_conversation(&chat).unwrap();

    assert_eq!(ids(&s.channels("t1").unwrap()), ["c2", "c1"]);
    assert_eq!(ids(&s.channels("t2").unwrap()), ["c3"]);
    assert!(s.channels("t9").unwrap().is_empty());
}

#[test]
fn kv_roundtrip() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.get_kv("epid").unwrap(), None);
    s.set_kv("epid", "abc").unwrap();
    assert_eq!(s.get_kv("epid").unwrap().as_deref(), Some("abc"));
    s.set_kv("epid", "def").unwrap();
    assert_eq!(s.get_kv("epid").unwrap().as_deref(), Some("def"));
    assert_eq!(s.get_kv("other").unwrap(), None);
}

#[test]
fn schema_version_and_indexes() {
    let s = Store::open_in_memory().unwrap();
    let v: i64 = s
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, migrations::MIGRATIONS_LEN as i64);
    let fk: i64 = s
        .conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
    let mut stmt = s
        .conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index'")
        .unwrap();
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    for want in [
        "idx_messages_conv_compose",
        "idx_messages_parent",
        "idx_conversations_last_activity",
    ] {
        assert!(names.iter().any(|n| n == want), "missing index {want}");
    }
}

fn temp_db_dir(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("kelma-store-test-{tag}-{}", std::process::id()))
}

#[test]
fn open_creates_dirs_and_reopen_keeps_data() {
    let dir = temp_db_dir("reopen");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("nested").join("deeper").join("cache.db");
    {
        let s = Store::open(&path).unwrap();
        assert!(path.exists());
        let mode: String = s
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        s.upsert_conversation(&conv("c", ConvKind::Group, "persist", 1))
            .unwrap();
        s.upsert_message(&msg("m", "c", 1)).unwrap();
        s.set_kv("k", "v").unwrap();
    }
    {
        let s = Store::open(&path).unwrap();
        assert_eq!(s.conversation("c").unwrap().unwrap().title, "persist");
        assert!(s.message("m").unwrap().is_some());
        assert_eq!(s.get_kv("k").unwrap().as_deref(), Some("v"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn open_rejects_newer_schema() {
    let dir = temp_db_dir("newer");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("cache.db");
    {
        let s = Store::open(&path).unwrap();
        s.conn.pragma_update(None, "user_version", 999).unwrap();
    }
    match Store::open(&path) {
        Err(Error::SchemaTooNew { found: 999, .. }) => {}
        Err(e) => panic!("unexpected error: {e}"),
        Ok(_) => panic!("expected SchemaTooNew"),
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- drafts ------------------------------------------------------------

#[test]
fn draft_roundtrip() {
    let s = Store::open_in_memory().unwrap();
    assert_eq!(s.draft("c").unwrap(), None);
    s.set_draft_at("c", "نص <b>hi</b>", Some("r1"), 10).unwrap();
    assert_eq!(
        s.draft("c").unwrap(),
        Some(Draft {
            conversation_id: "c".into(),
            text: "نص <b>hi</b>".into(),
            reply_to: Some("r1".into()),
            updated_ms: 10,
        })
    );
    // Overwrite keeps one row and replaces the reply target.
    s.set_draft_at("c", "second", None, 20).unwrap();
    let d = s.draft("c").unwrap().unwrap();
    assert_eq!(
        (d.text.as_str(), d.reply_to, d.updated_ms),
        ("second", None, 20)
    );
    assert_eq!(s.drafts().unwrap().len(), 1);
}

#[test]
fn set_draft_stamps_current_time() {
    let s = Store::open_in_memory().unwrap();
    s.set_draft("c", "x", None).unwrap();
    assert!(s.draft("c").unwrap().unwrap().updated_ms > 1_600_000_000_000);
}

#[test]
fn empty_draft_deletes() {
    let s = Store::open_in_memory().unwrap();
    s.set_draft_at("c", "x", None, 1).unwrap();
    s.set_draft_at("c", "", Some("r"), 2).unwrap();
    assert_eq!(s.draft("c").unwrap(), None);
    assert!(s.drafts().unwrap().is_empty());
    // Deleting a draft that does not exist is fine.
    s.set_draft("nope", "", None).unwrap();
}

#[test]
fn drafts_newest_first() {
    let s = Store::open_in_memory().unwrap();
    s.set_draft_at("a", "1", None, 10).unwrap();
    s.set_draft_at("b", "2", None, 30).unwrap();
    s.set_draft_at("c", "3", None, 20).unwrap();
    s.set_draft_at("a", "1b", None, 40).unwrap();
    let order: Vec<_> = s
        .drafts()
        .unwrap()
        .into_iter()
        .map(|d| d.conversation_id)
        .collect();
    assert_eq!(order, ["a", "b", "c"]);
}

// ---- outbox ------------------------------------------------------------

fn item(id: &str, conv: &str, created: i64) -> OutboxItem {
    OutboxItem {
        client_message_id: id.into(),
        conversation_id: conv.into(),
        html: format!("<p>{id}</p>"),
        subject: None,
        reply_to: None,
        kind: OutboxKind::Text,
        payload_path: None,
        created_ms: created,
        attempts: 0,
        state: OutboxState::Queued,
        last_error: None,
    }
}

fn outbox_ids(v: &[OutboxItem]) -> Vec<&str> {
    v.iter().map(|i| i.client_message_id.as_str()).collect()
}

#[test]
fn outbox_enqueue_roundtrips_all_fields_and_orders_due_oldest_first() {
    let s = Store::open_in_memory().unwrap();
    let mut file = item("f", "c1", 5);
    file.kind = OutboxKind::File;
    file.payload_path = Some("/tmp/a.pdf".into());
    file.subject = Some("subj".into());
    file.reply_to = Some("root".into());
    s.enqueue(&item("late", "c1", 30)).unwrap();
    s.enqueue(&file).unwrap();
    s.enqueue(&item("mid", "c2", 10)).unwrap();
    let due = s.outbox_due().unwrap();
    assert_eq!(outbox_ids(&due), ["f", "mid", "late"]);
    assert_eq!(due[0], file);
}

#[test]
fn every_outbox_kind_roundtrips() {
    let s = Store::open_in_memory().unwrap();
    for (i, k) in [OutboxKind::Text, OutboxKind::Image, OutboxKind::File]
        .into_iter()
        .enumerate()
    {
        let mut it = item(&format!("k{i}"), "c", i as i64);
        it.kind = k;
        s.enqueue(&it).unwrap();
        assert_eq!(s.outbox_for("c").unwrap()[i].kind, k);
    }
}

#[test]
fn outbox_enqueue_forces_fresh_state() {
    let s = Store::open_in_memory().unwrap();
    let mut i = item("a", "c", 1);
    i.attempts = 7;
    i.state = OutboxState::Failed;
    i.last_error = Some("old".into());
    s.enqueue(&i).unwrap();
    let got = &s.outbox_for("c").unwrap()[0];
    assert_eq!(got.state, OutboxState::Queued);
    assert_eq!((got.attempts, got.last_error.as_deref()), (0, None));
}

#[test]
fn outbox_sending_and_failure_leave_due() {
    let s = Store::open_in_memory().unwrap();
    s.enqueue(&item("a", "c", 1)).unwrap();
    s.enqueue(&item("b", "c", 2)).unwrap();
    s.enqueue(&item("c3", "c", 3)).unwrap();
    s.mark_sending("a").unwrap();
    s.mark_sending("b").unwrap();
    s.mark_failed("b", "boom").unwrap();
    assert_eq!(outbox_ids(&s.outbox_due().unwrap()), ["c3"]);

    let all = s.outbox_for("c").unwrap();
    assert_eq!(outbox_ids(&all), ["a", "b", "c3"]);
    assert_eq!(all[0].state, OutboxState::Sending);
    assert_eq!(all[1].state, OutboxState::Failed);
    assert_eq!(all[1].attempts, 1);
    assert_eq!(all[1].last_error.as_deref(), Some("boom"));
}

#[test]
fn outbox_retry_requeues_failed_and_keeps_attempts() {
    let s = Store::open_in_memory().unwrap();
    s.enqueue(&item("a", "c", 1)).unwrap();
    s.mark_failed("a", "e1").unwrap();
    s.retry("a").unwrap();
    assert_eq!(outbox_ids(&s.outbox_due().unwrap()), ["a"]);
    s.mark_failed("a", "e2").unwrap();
    let got = s.outbox_for("c").unwrap().remove(0);
    assert_eq!(got.attempts, 2);
    assert_eq!(got.last_error.as_deref(), Some("e2"));
    // Retry only applies to failed rows.
    s.retry("a").unwrap();
    s.mark_sending("a").unwrap();
    s.retry("a").unwrap();
    assert_eq!(s.outbox_for("c").unwrap()[0].state, OutboxState::Sending);
}

#[test]
fn outbox_remove_and_per_conversation_listing() {
    let s = Store::open_in_memory().unwrap();
    s.enqueue(&item("a", "c1", 1)).unwrap();
    s.enqueue(&item("b", "c2", 2)).unwrap();
    assert_eq!(outbox_ids(&s.outbox_for("c1").unwrap()), ["a"]);
    s.remove("a").unwrap();
    assert!(s.outbox_for("c1").unwrap().is_empty());
    assert_eq!(outbox_ids(&s.outbox_due().unwrap()), ["b"]);
    s.remove("a").unwrap();
}

#[test]
fn outbox_survives_restart_and_sending_resets_to_queued() {
    let dir = temp_db_dir("outbox");
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("cache.db");
    {
        let s = Store::open(&path).unwrap();
        s.enqueue(&item("crashed", "c", 1)).unwrap();
        s.enqueue(&item("failed", "c", 2)).unwrap();
        s.enqueue(&item("queued", "c", 3)).unwrap();
        s.mark_sending("crashed").unwrap();
        s.mark_failed("failed", "net").unwrap();
    }
    {
        let s = Store::open(&path).unwrap();
        let due = s.outbox_due().unwrap();
        assert_eq!(outbox_ids(&due), ["crashed", "queued"]);
        let failed = &s.outbox_for("c").unwrap()[1];
        assert_eq!(failed.state, OutboxState::Failed);
        assert_eq!(failed.last_error.as_deref(), Some("net"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn migrates_from_previous_schema_version() {
    let dir = temp_db_dir("migrate");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cache.db");
    {
        // A database exactly as the previous release left it (v1).
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(migrations::MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO kv (key, value) VALUES ('epid', 'e')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO conversations VALUES ('c', 'group', 't', NULL, NULL, 1, '', 0, 0, 0)",
            [],
        )
        .unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.get_kv("epid").unwrap().as_deref(), Some("e"));
    assert!(s.conversation("c").unwrap().is_some());
    s.set_draft("c", "x", None).unwrap();
    s.enqueue(&item("a", "c", 1)).unwrap();
    assert!(s.draft("c").unwrap().is_some());
    assert_eq!(s.outbox_due().unwrap().len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
