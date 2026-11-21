//! Demo mode (`IRONTEAMS_DEMO=1`): no network, a seeded in-memory store. Used for screenshots,
//! UI work and memory measurements without a Teams account.

use serde_json::json;
use store::{ConvKind, ConversationRow, MessageRow, Store, TeamRow, UserRow};
use teams_proto::activity::{ActivityItem, ActivityKind};

pub const ME: &str = "8:orgid:00000000-0000-0000-0000-000000000001";

fn user(n: u8) -> String {
    format!("8:orgid:00000000-0000-0000-0000-0000000000{n:02}")
}

/// Fills `st` with sample chats, channels and messages. `now` is Unix ms.
pub fn seed(st: &Store, now: i64) -> anyhow::Result<()> {
    let people = [
        ("Ahmed Ali", 2),
        ("Sara Hassan", 3),
        ("Omar Khaled", 4),
        ("Mona Adel", 5),
        ("Youssef Samir", 6),
    ];
    for (name, n) in people {
        st.upsert_user(&UserRow {
            mri: user(n),
            display_name: name.into(),
            email: None,
            avatar: None,
            updated_ms: now,
        })?;
    }
    let min = 60_000;
    let day = 24 * 60 * min;
    let conv = |id: &str,
                kind,
                title: &str,
                team: Option<(&str, &str)>,
                ago: i64,
                preview: &str,
                unread: bool| ConversationRow {
        id: id.into(),
        kind,
        title: title.into(),
        team_id: team.map(|t| t.0.into()),
        team_name: team.map(|t| t.1.into()),
        last_activity_ms: now - ago,
        last_preview: preview.into(),
        unread,
        favorite: false,
        read_horizon_ms: 0,
    };
    let team = ("19:team1@thread.tacv2", "Contoso Ltd.");
    st.upsert_team(&TeamRow {
        id: team.0.into(),
        name: team.1.into(),
    })?;
    let rows = [
        conv(
            "48:notes",
            ConvKind::Notes,
            "Mohamed Eslam (You)",
            None,
            3 * min,
            "You: contoso-docs-demo.mp4",
            false,
        ),
        conv(
            "19:backend@thread.v2",
            ConvKind::Group,
            "Backend Team",
            None,
            5 * min,
            "Sara: مين فاضي يعمل review؟ 🙏",
            true,
        ),
        conv(
            "19:00000000-0000-0000-0000-000000000001_00000000-0000-0000-0000-000000000002@unq.gbl.spaces",
            ConvKind::OneOnOne,
            "Ahmed Ali",
            None,
            40 * min,
            "تمام، I'll update the ticket",
            false,
        ),
        conv(
            "19:00000000-0000-0000-0000-000000000001_00000000-0000-0000-0000-000000000003@unq.gbl.spaces",
            ConvKind::OneOnOne,
            "Sara Hassan",
            None,
            2 * 60 * min,
            "LGTM, merging now. 🚀",
            true,
        ),
        conv(
            "19:standup@thread.v2",
            ConvKind::Group,
            "Morning Standup",
            None,
            day,
            "Omar: Blocked on the Redis upgrade",
            false,
        ),
        conv(
            "19:admin@thread.v2",
            ConvKind::Group,
            "مجموعة الإدارة",
            None,
            2 * day,
            "اجتماع بكرة الساعة ١٠",
            false,
        ),
        conv(
            "19:prodbugs@thread.v2",
            ConvKind::Group,
            "Production Bugs",
            None,
            3 * day,
            "Youssef: Fixed in 2.4.1",
            false,
        ),
        conv(
            "19:team1@thread.tacv2",
            ConvKind::Channel,
            "General",
            Some(team),
            30 * min,
            "Release 2.4 is live 🎉",
            true,
        ),
        conv(
            "19:tech@thread.tacv2",
            ConvKind::Channel,
            "tech only",
            Some(team),
            4 * day,
            "Postgres 17 upgrade plan",
            false,
        ),
    ];
    for r in &rows {
        st.upsert_conversation(r)?;
    }
    st.set_favorite("48:notes", true)?;
    st.set_favorite("19:backend@thread.v2", true)?;

    let mut id = 1_000_000i64;
    let mut msg = |conv: &str,
                   parent: Option<&str>,
                   from: &str,
                   name: &str,
                   ago: i64,
                   html: &str,
                   props: serde_json::Value|
     -> anyhow::Result<String> {
        id += 1;
        let mid = (now - ago + id % 1000).to_string();
        st.upsert_message(&MessageRow {
            id: mid.clone(),
            conversation_id: conv.into(),
            parent_id: parent.map(str::to_owned),
            sender_mri: from.into(),
            sender_name: name.into(),
            compose_ms: now - ago,
            edited_ms: None,
            deleted: false,
            message_type: "RichText/Html".into(),
            content_html: html.into(),
            client_message_id: None,
            reactions_json: props.to_string(),
        })?;
        Ok(mid)
    };
    let c = "19:backend@thread.v2";
    let none = json!({});
    msg(
        c,
        None,
        &user(2),
        "Ahmed Ali",
        day + 3 * 60 * min,
        "<p>صباح الخير يا جماعة، الـ deploy خلص؟</p>",
        none.clone(),
    )?;
    msg(
        c,
        None,
        &user(3),
        "Sara Hassan",
        day + 170 * min,
        "<p>Good morning! Yes, it was deployed to <b>staging</b> at 10:30.</p>",
        json!({"r":[{"key":"like","users":[ME, user(2)]}]}),
    )?;
    msg(
        c,
        None,
        ME,
        "Mohamed Eslam",
        50 * min,
        "<p>تمام 👍 هراجع الـ PR بتاع <a href=\"https://github.com\">CPS-122</a> النهارده</p>",
        none.clone(),
    )?;
    msg(
        c,
        None,
        &user(4),
        "Omar Khaled",
        45 * min,
        "<p>Can you check the Redis memory graph? It spiked to <b>80%</b> after the release.</p><ul><li>eu-west-1</li><li>me-central-1</li></ul>",
        json!({"r":[{"key":"surprised","users":[user(2)]}]}),
    )?;
    msg(
        c,
        None,
        ME,
        "Mohamed Eslam",
        44 * min,
        "<p>الـ API بترجع 500 لما الـ token يخلص، محتاجين refresh logic قبل ما نعمل release.</p>",
        json!({"r":[{"key":"heart","users":[user(3)]},{"key":"like","users":[user(4), user(2)]}]}),
    )?;
    msg(
        c,
        None,
        &user(3),
        "Sara Hassan",
        30 * min,
        &render::reply_html(
            "1",
            ME,
            "Mohamed Eslam",
            now - 44 * min,
            "الـ API بترجع 500 لما الـ token يخلص",
            "I'll add the retry in the auth middleware today.",
        ),
        none.clone(),
    )?;
    msg(
        c,
        None,
        &user(4),
        "Omar Khaled",
        12 * min,
        "<p>بالنسبة للـ architecture اللي اتكلمنا عليها امبارح، أنا شايف إن الأحسن نفصل الـ worker عن الـ API server عشان لو الـ queue اتملت ما تأثرش على الـ response time.</p>",
        none.clone(),
    )?;
    msg(
        c,
        None,
        &user(3),
        "Sara Hassan",
        5 * min,
        "<p>مين فاضي يعمل review؟ 🙏</p>",
        none.clone(),
    )?;

    let notes = "48:notes";
    msg(
        notes,
        None,
        ME,
        "Mohamed Eslam",
        day + 60 * min,
        "<p><a href=\"https://contoso-my.sharepoint.com/\">https://contoso-my.sharepoint.com/:v:/g/personal/demo</a></p><p>archive voice note</p>",
        json!({"f":[{"name":"archive voice note.m4a","url":"https://contoso-my.sharepoint.com/","type":"m4a"}]}),
    )?;
    msg(
        notes,
        None,
        ME,
        "Mohamed Eslam",
        3 * min,
        "<p>contoso-docs-demo.mp4</p>",
        json!({"f":[{"name":"contoso-docs-demo.mp4","url":"https://contoso-my.sharepoint.com/","type":"mp4"}]}),
    )?;

    let ch = "19:team1@thread.tacv2";
    let root = msg(
        ch,
        None,
        &user(5),
        "Mona Adel",
        2 * 60 * min,
        "<p>Release 2.4 is live 🎉 Thanks everyone for the hard work this sprint.</p>",
        json!({"s":"Release 2.4","r":[{"key":"like","users":[ME, user(2), user(3), user(4)]}]}),
    )?;
    msg(
        ch,
        Some(&root),
        &user(2),
        "Ahmed Ali",
        100 * min,
        "<p>مبروك يا جماعة 🎉</p>",
        none.clone(),
    )?;
    msg(
        ch,
        Some(&root),
        ME,
        "Mohamed Eslam",
        90 * min,
        "<p>Great work! The new docs site is next.</p>",
        none.clone(),
    )?;
    msg(
        ch,
        None,
        &user(6),
        "Youssef Samir",
        30 * min,
        "<p>Reminder: all-hands on Thursday at 3 PM.</p>",
        json!({"s":"All-hands"}),
    )?;
    Ok(())
}

/// Invented events for the demo calendar, as a Graph `calendarView` page. `week_start_ms` is local
/// Monday 00:00 this week; one event starts three minutes after `now_ms` so the Join banner shows.
pub fn calendar_events(week_start_ms: i64, now_ms: i64) -> serde_json::Value {
    let min = 60_000;
    let day = 24 * 60 * min;
    let at = |d: i64, h: i64, m: i64| week_start_ms + d * day + (h * 60 + m) * min;
    let join = "https://teams.microsoft.com/l/meetup-join/demo";
    let mut n = 0;
    let mut ev = |subject: &str, from: i64, to: i64, extra: serde_json::Value| {
        n += 1;
        let t = |ms| json!({"dateTime": calendar::format_iso_ms(ms), "timeZone": "UTC"});
        let mut v = json!({"id": format!("demo-{n}"), "subject": subject,
            "start": t(from), "end": t(to),
            "organizer": {"emailAddress": {"name": "Sara Hassan", "address": "sara@example.com"}}});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    let online = json!({"isOnlineMeeting": true, "onlineMeeting": {"joinUrl": join}});
    let mut items = Vec::new();
    for d in 0..5 {
        let mut extra = online.as_object().unwrap().clone();
        extra.insert("type".into(), "occurrence".into());
        items.push(ev("Team standup", at(d, 9, 30), at(d, 9, 45), extra.into()));
    }
    let mut design = online.as_object().unwrap().clone();
    design.insert(
        "organizer".into(),
        json!({"emailAddress": {"name": "Ahmed Ali", "address": "ahmed@example.com"}}),
    );
    // An invitation I have not answered, so the peek shows the answer buttons.
    let who = |name: &str, mail: &str, kind: &str, response: &str| {
        json!({"emailAddress": {"name": name, "address": mail}, "type": kind,
            "status": {"response": response}})
    };
    design.insert("responseStatus".into(), json!({"response": "notResponded"}));
    design.insert(
        "attendees".into(),
        json!([
            who("Demo User", "demo@contoso.com", "required", "notResponded"),
            who("Sara Hassan", "sara@example.com", "required", "accepted"),
            who("Omar Khaled", "omar@example.com", "optional", "declined"),
            who(
                "Mona Adel",
                "mona@example.com",
                "required",
                "tentativelyAccepted"
            ),
        ]),
    );
    items.push(ev(
        "Design review",
        at(1, 10, 0),
        at(1, 11, 0),
        design.into(),
    ));
    items.push(ev(
        "1:1 with Omar",
        at(1, 10, 30),
        at(1, 11, 30),
        json!({"organizer": {"emailAddress": {"name": "Omar Khaled", "address": "omar@example.com"}}}),
    ));
    items.push(ev(
        "Vendor call",
        at(1, 10, 45),
        at(1, 11, 15),
        json!({"showAs": "tentative", "location": {"displayName": "Phone"}}),
    ));
    items.push(ev(
        "Sprint review",
        at(2, 11, 0),
        at(2, 12, 0),
        json!({"showAs": "tentative", "categories": ["Green category"],
            "responseStatus": {"response": "organizer"},
            "attendees": [who("Sara Hassan", "sara@example.com", "required", "accepted"),
                who("Ahmed Ali", "ahmed@example.com", "required", "notResponded")]}),
    ));
    items.push(ev(
        "Lunch with Mona",
        at(2, 12, 30),
        at(2, 13, 30),
        json!({"showAs": "free", "location": {"displayName": "Cafe"}}),
    ));
    items.push(ev(
        "Quarterly planning",
        at(2, 14, 0),
        at(2, 15, 0),
        json!({"responseStatus": {"response": "declined"}}),
    ));
    items.push(ev(
        "Interview: backend engineer",
        at(0, 16, 0),
        at(0, 17, 0),
        json!({"sensitivity": "private", "categories": ["Purple category"]}),
    ));
    items.push(ev(
        "Product conference",
        at(0, 0, 0),
        at(2, 0, 0),
        json!({"isAllDay": true, "categories": ["Blue category"]}),
    ));
    items.push(ev(
        "Working from home",
        at(3, 0, 0),
        at(4, 0, 0),
        json!({"isAllDay": true, "showAs": "workingElsewhere"}),
    ));
    items.push(ev(
        "Annual leave",
        at(4, 0, 0),
        at(5, 0, 0),
        json!({"isAllDay": true, "showAs": "oof"}),
    ));
    let soon = now_ms / min * min + 3 * min;
    items.push(ev("Customer sync", soon, soon + 30 * min, online));
    json!({ "value": items })
}

/// Invented Activity feed over the seeded conversations, newest first. `now` is Unix ms.
pub fn activity(st: &Store, now: i64) -> Vec<ActivityItem> {
    let backend = "19:backend@thread.v2";
    let channel = "19:team1@thread.tacv2";
    let min = 60_000;
    let nth = |conv: &str, n: usize| {
        st.messages_before(conv, None, 100)
            .ok()
            .and_then(|v| v.get(n).map(|m| m.id.clone()))
            .unwrap_or_default()
    };
    let root = nth(channel, 0);
    let reply = st
        .thread_replies(&root, 10)
        .ok()
        .and_then(|v| v.first().map(|m| m.id.clone()))
        .unwrap_or_default();
    let item = |n: u8,
                kind: ActivityKind,
                who: (&str, u8),
                thread: &str,
                msg: String,
                chain: String,
                preview: &str,
                ago: i64,
                read: bool| ActivityItem {
        id: (now - ago).to_string(),
        kind,
        source_thread_id: thread.into(),
        source_message_id: msg,
        source_reply_chain_id: chain,
        actor_mri: user(who.1),
        actor_name: who.0.into(),
        thread_topic: String::new(),
        preview: preview.into(),
        timestamp_ms: now - ago + i64::from(n),
        is_read: read,
    };
    let review = nth(backend, 7);
    let redis = nth(backend, 3);
    let token = nth(backend, 4);
    let all_hands = nth(channel, 1);
    vec![
        item(
            1,
            ActivityKind::Mention,
            ("Sara Hassan", 3),
            backend,
            review.clone(),
            review,
            "@Mohamed can you review the retry change?",
            4 * min,
            false,
        ),
        item(
            2,
            ActivityKind::Reaction {
                emoji_key: "heart".into(),
            },
            ("Omar Khaled", 4),
            backend,
            token.clone(),
            token,
            "الـ API بترجع 500 لما الـ token يخلص",
            20 * min,
            false,
        ),
        item(
            3,
            ActivityKind::ChannelNewMessage,
            ("Youssef Samir", 6),
            channel,
            all_hands.clone(),
            all_hands,
            "All-hands: Reminder: all-hands on Thursday at 3 PM.",
            30 * min,
            false,
        ),
        item(
            4,
            ActivityKind::Reply,
            ("Ahmed Ali", 2),
            channel,
            reply,
            root.clone(),
            "مبروك يا جماعة 🎉",
            100 * min,
            true,
        ),
        item(
            5,
            ActivityKind::TagMention,
            ("Mona Adel", 5),
            channel,
            root.clone(),
            root,
            "Release 2.4: @Backend thanks everyone for the hard work",
            2 * 60 * min,
            true,
        ),
        item(
            6,
            ActivityKind::EveryoneMention,
            ("Omar Khaled", 4),
            backend,
            redis.clone(),
            redis,
            "@Everyone can you check the Redis memory graph?",
            24 * 60 * min,
            true,
        ),
    ]
}

// -- OneDrive demo data ------------------------------------------------------------------------

/// The libraries under Quick access: `(drive id, name)`.
pub fn drive_libraries() -> Vec<(String, String)> {
    vec![
        ("b!sales".into(), "Sales team".into()),
        ("b!design".into(), "Design system".into()),
        ("b!ops".into(), "Operations".into()),
    ]
}

/// Ids of the demo items that start as favorites.
pub fn drive_favorites() -> std::collections::HashSet<String> {
    ["demo-budget", "demo-projects", "demo-brand"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn drive_file(
    id: &str,
    name: &str,
    size: u64,
    ago_min: i64,
    by: &str,
    dir: &str,
    now: i64,
) -> onedrive::DriveItem {
    onedrive::DriveItem {
        id: id.into(),
        drive_id: "b!demo".into(),
        name: name.into(),
        size,
        modified_ms: now - ago_min * 60_000,
        modified_by: by.into(),
        created_by: by.into(),
        parent_path: dir.into(),
        web_url: format!("https://contoso-my.example/personal/demo/Documents{dir}/{name}"),
        ..Default::default()
    }
}

fn drive_dir(
    id: &str,
    name: &str,
    kids: u32,
    ago_min: i64,
    by: &str,
    now: i64,
) -> onedrive::DriveItem {
    onedrive::DriveItem {
        is_folder: true,
        child_count: kids,
        ..drive_file(id, name, 0, ago_min, by, "", now)
    }
}

/// A folder created in demo mode.
pub fn drive_new_folder(name: &str) -> onedrive::DriveItem {
    drive_dir(
        &format!("demo-new-{name}"),
        name,
        0,
        0,
        "Mohamed Eslam",
        teams_proto::time::now_ms(),
    )
}

/// What a demo section or folder lists. `key` is a section key (`home`, `files`, `shared`,
/// `favorites`, `libraries`, `bin`, `meetings`, `drive:<id>`, `search`) and `folder` the open
/// folder id of `files` (`root` or empty for the top).
pub fn drive_listing(key: &str, folder: &str, now: i64) -> Vec<onedrive::DriveItem> {
    let f = |id, name, size, ago, by, dir| drive_file(id, name, size, ago, by, dir, now);
    let d = |id, name, kids, ago, by| drive_dir(id, name, kids, ago, by, now);
    let me = "Mohamed Eslam";
    match (key, folder) {
        ("files", "" | "root") => vec![
            d("demo-projects", "Projects", 4, 60 * 30, me),
            d("demo-pictures", "Pictures", 12, 60 * 24 * 6, me),
            d("demo-brand", "Brand assets", 7, 60 * 24 * 20, me),
            f("demo-budget", "Budget 2026.xlsx", 3_400_000, 45, me, ""),
            f("demo-q4", "Q4 plan.docx", 84_000, 60 * 3, me, ""),
            f(
                "demo-offsite",
                "Team offsite.pptx",
                12_600_000,
                60 * 30,
                "Sara Hassan",
                "",
            ),
            f("demo-notes", "Meeting notes.md", 1_800, 60 * 26, me, ""),
            f("demo-readme", "README.txt", 640, 60 * 24 * 40, me, ""),
            f(
                "demo-contract",
                "Vendor contract.pdf",
                640_000,
                60 * 24 * 8,
                "Omar Khaled",
                "",
            ),
            f("demo-logo", "logo.png", 38_000, 60 * 24 * 90, me, ""),
        ],
        ("files", "demo-projects") => vec![
            d("demo-alpha", "Alpha", 3, 60 * 5, me),
            d("demo-beta", "Beta", 0, 60 * 24 * 12, me),
            f(
                "demo-kickoff",
                "Kickoff.pptx",
                4_200_000,
                60 * 24 * 3,
                "Ahmed Ali",
                "/Projects",
            ),
        ],
        ("files", "demo-alpha") => vec![
            f(
                "demo-spec",
                "Spec.docx",
                56_000,
                60 * 5,
                me,
                "/Projects/Alpha",
            ),
            f(
                "demo-timeline",
                "Timeline.xlsx",
                98_000,
                60 * 9,
                "Mona Adel",
                "/Projects/Alpha",
            ),
            f(
                "demo-script",
                "build.sh",
                920,
                60 * 24 * 2,
                me,
                "/Projects/Alpha",
            ),
        ],
        ("files", _) => Vec::new(),
        ("home", _) => vec![
            f("demo-budget", "Budget 2026.xlsx", 3_400_000, 45, me, "/"),
            f(
                "demo-pitch",
                "Customer pitch.pptx",
                9_800_000,
                60 * 2,
                "Sara Hassan",
                "/Sales",
            ),
            f("demo-q4", "Q4 plan.docx", 84_000, 60 * 3, me, "/"),
            f(
                "demo-contract",
                "Vendor contract.pdf",
                640_000,
                60 * 24,
                "Omar Khaled",
                "/Legal",
            ),
            f(
                "demo-roadmap",
                "Roadmap.xlsx",
                210_000,
                60 * 26,
                "Youssef Samir",
                "/Product",
            ),
            f("demo-notes", "Meeting notes.md", 1_800, 60 * 28, me, "/"),
            f(
                "demo-spec",
                "Spec.docx",
                56_000,
                60 * 24 * 3,
                me,
                "/Projects/Alpha",
            ),
            f(
                "demo-demo",
                "Demo recording.mp4",
                48_000_000,
                60 * 24 * 5,
                "Mona Adel",
                "/Recordings",
            ),
        ],
        ("shared", _) => vec![
            f(
                "demo-pitch",
                "Customer pitch.pptx",
                9_800_000,
                60 * 2,
                "Sara Hassan",
                "",
            ),
            f(
                "demo-roadmap",
                "Roadmap.xlsx",
                210_000,
                60 * 26,
                "Youssef Samir",
                "",
            ),
            f(
                "demo-contract",
                "Vendor contract.pdf",
                640_000,
                60 * 24,
                "Omar Khaled",
                "",
            ),
            d(
                "demo-handbook",
                "Team handbook",
                9,
                60 * 24 * 14,
                "Mona Adel",
            ),
        ],
        ("favorites", _) => vec![
            f("demo-budget", "Budget 2026.xlsx", 3_400_000, 45, me, "/"),
            d("demo-projects", "Projects", 4, 60 * 30, me),
            d("demo-brand", "Brand assets", 7, 60 * 24 * 20, me),
        ],
        ("libraries", _) => drive_libraries()
            .into_iter()
            .map(|(id, name)| onedrive::DriveItem {
                id: "root".into(),
                drive_id: id,
                name,
                is_folder: true,
                ..Default::default()
            })
            .collect(),
        ("bin", _) => vec![
            f("bin-1", "Old draft.docx", 31_000, 60 * 24, me, "/Documents"),
            f("bin-2", "Scratch.txt", 200, 60 * 24 * 2, me, "/Documents"),
            d("bin-3", "Archive 2024", 14, 60 * 24 * 9, me),
        ],
        ("meetings", _) => vec![
            f(
                "demo-rec-1",
                "Weekly sync-20261112.mp4",
                212_000_000,
                60 * 24 * 6,
                me,
                "/Recordings",
            ),
            f(
                "demo-rec-2",
                "Design review-20261105.mp4",
                156_000_000,
                60 * 24 * 13,
                me,
                "/Recordings",
            ),
        ],
        ("shared-by", _) => vec![
            f("demo-by-1", "Budget 2026.xlsx", 3_400_000, 60 * 5, "", "/"),
            f("demo-by-2", "Q4 plan.docx", 84_000, 60 * 24 * 2, "", "/"),
            d("demo-by-3", "Brand assets", 7, 60 * 24 * 8, ""),
        ],
        ("media", _) => vec![
            f(
                "demo-img-1",
                "Offsite group photo.jpg",
                4_200_000,
                60 * 24 * 3,
                me,
                "/Pictures",
            ),
            f(
                "demo-img-2",
                "Whiteboard.png",
                900_000,
                60 * 24 * 5,
                me,
                "/Pictures",
            ),
            f(
                "demo-vid-1",
                "Demo walkthrough.mp4",
                88_000_000,
                60 * 24 * 9,
                me,
                "/Pictures",
            ),
        ],
        ("search", _) => ["home", "shared"]
            .into_iter()
            .flat_map(|k| drive_listing(k, "", now))
            .collect(),
        (k, _) if k.starts_with("drive:") => vec![
            d("demo-lib-docs", "Documents", 6, 60 * 24, "Ahmed Ali"),
            f(
                "demo-lib-price",
                "Price list.xlsx",
                74_000,
                60 * 48,
                "Sara Hassan",
                "",
            ),
            f(
                "demo-lib-brief",
                "Brief.docx",
                33_000,
                60 * 72,
                "Ahmed Ali",
                "",
            ),
        ],
        _ => Vec::new(),
    }
}

/// A minimal valid PDF with one page per `(width, height)` in points, each with a black square
/// in its top-left quarter (what the demo shows for a PDF or an Office file).
pub fn drive_pdf(pages: &[(u32, u32)]) -> Vec<u8> {
    let n = pages.len();
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        format!(
            "<< /Type /Pages /Count {n} /Kids [{}] >>",
            (0..n)
                .map(|i| format!("{} 0 R", 3 + 2 * i))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    for (i, (w, h)) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>",
            4 + 2 * i
        ));
        let content = format!("0 0 0 rg 0 {} {} {} re f", h / 2, w / 2, h / 2);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// The recordings kept in a team channel's library, which Meetings adds in the background.
pub fn drive_channel_recordings(now: i64) -> Vec<onedrive::DriveItem> {
    let mut item = drive_file(
        "demo-rec-3",
        "Sprint review-20261110.mp4",
        340_000_000,
        60 * 24 * 8,
        "Sara Hassan",
        "",
        now,
    );
    item.drive_id = "b!sales".into();
    item.parent_path = "Sales team \u{203a} General".into();
    vec![item]
}

/// Home's "For you" cards.
pub fn drive_for_you(now: i64) -> Vec<onedrive::ForYouCard> {
    use onedrive::{ForYouCard, Insight, InsightKind, ItemRef, Person};
    let card = |id: &str, title: &str, kind, by: Option<&str>, reason: &str, ago: i64| ForYouCard {
        insight: Insight {
            kind,
            title: title.into(),
            item: Some(ItemRef::new("b!demo", id)),
            when_ms: now - ago * 60_000,
            shared_by: by.map(|n| Person {
                name: n.into(),
                ..Default::default()
            }),
            ..Default::default()
        },
        reason: reason.into(),
    };
    vec![
        card(
            "demo-card-1",
            "Team offsite.pptx",
            InsightKind::Shared,
            Some("Sara Hassan"),
            "Sara Hassan shared this in a Teams chat",
            60 * 30,
        ),
        card(
            "demo-budget",
            "Budget 2026.xlsx",
            InsightKind::Used,
            None,
            "You frequently open this",
            45,
        ),
        card(
            "demo-card-3",
            "Roadmap.docx",
            InsightKind::Trending,
            None,
            "Trending around you",
            60 * 6,
        ),
    ]
}

/// "Browse files by People": who shared files with the user, newest share first.
pub fn drive_people_files(now: i64) -> Vec<onedrive::PersonFiles> {
    use onedrive::{Person, PersonFiles};
    let person = |name: &str| Person {
        id: format!("demo-{name}"),
        name: name.into(),
        address: String::new(),
    };
    let share = |id, name, ago, by: &str| drive_file(id, name, 20_000, ago, by, "", now);
    let group = |name: &str, files: Vec<onedrive::DriveItem>| PersonFiles {
        latest_ms: files.iter().map(|f| f.modified_ms).max().unwrap_or(0),
        person: person(name),
        files,
    };
    vec![
        group(
            "Sara Hassan",
            vec![
                share("demo-sh-1", "Team offsite.pptx", 60 * 30, "Sara Hassan"),
                share("demo-sh-2", "Price list.xlsx", 60 * 24 * 4, "Sara Hassan"),
            ],
        ),
        group(
            "Ahmed Ali",
            vec![share("demo-sh-3", "Brief.docx", 60 * 72, "Ahmed Ali")],
        ),
    ]
}

/// Text shown when a demo text or markdown file is opened.
pub fn drive_file_text(name: &str) -> String {
    if name.to_lowercase().ends_with(".md") {
        "# Meeting notes\n\n**Date:** Wednesday\n\n- Review the Q4 plan\n- Confirm the vendor contract\n- Decide the offsite date\n\nThe next sync is on Monday.".into()
    } else {
        format!("{name}\n\nThis is demo text. Nothing here is stored.")
    }
}

pub fn drive_versions(now: i64) -> Vec<onedrive::Version> {
    [
        (0, "Mohamed Eslam", 84_000),
        (60 * 26, "Mona Adel", 82_500),
        (60 * 24 * 9, "Mohamed Eslam", 61_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(n, (ago, by, size))| onedrive::Version {
        id: format!("{}.0", 3 - n),
        modified_ms: now - ago * 60_000,
        modified_by: by.into(),
        size,
    })
    .collect()
}

pub fn drive_permissions() -> Vec<onedrive::Permission> {
    vec![
        onedrive::Permission {
            id: "perm-owner".into(),
            roles: vec!["owner".into()],
            granted_to: vec!["Mohamed Eslam".into()],
            ..Default::default()
        },
        onedrive::Permission {
            id: "perm-sara".into(),
            roles: vec!["write".into()],
            granted_to: vec!["Sara Hassan".into()],
            ..Default::default()
        },
        onedrive::Permission {
            id: "perm-link".into(),
            roles: vec!["read".into()],
            link_scope: Some("organization".into()),
            link_type: Some("view".into()),
            ..Default::default()
        },
    ]
}

/// Directory people matching `q`, for the Share dialog's picker.
pub fn drive_people(q: &str) -> Vec<teams_proto::people::Person> {
    let q = q.to_lowercase();
    [
        ("Ahmed Ali", 2, "Engineering manager"),
        ("Sara Hassan", 3, "Sales lead"),
        ("Omar Khaled", 4, "Legal counsel"),
        ("Mona Adel", 5, "Designer"),
        ("Youssef Samir", 6, "Product manager"),
    ]
    .into_iter()
    .filter(|(name, _, _)| name.to_lowercase().contains(&q))
    .map(|(name, n, title)| teams_proto::people::Person {
        mri: user(n),
        display_name: name.into(),
        email: Some(format!(
            "{}@contoso.example",
            name.to_lowercase().replace(' ', ".")
        )),
        job_title: Some(title.into()),
        department: None,
    })
    .collect()
}
