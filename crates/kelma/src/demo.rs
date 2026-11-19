//! Demo mode (`KELMA_DEMO=1`): no network, a seeded in-memory store. Used for screenshots,
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
    let team = ("19:team1@thread.tacv2", "Velents Technologies Inc.");
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
            "You: velents-docs-demo.mp4",
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
        "<p><a href=\"https://velentstechnologiesinc-my.sharepoint.com/\">https://velentstechnologiesinc-my.sharepoint.com/:v:/g/personal/demo</a></p><p>Safha-archive Saudi voice</p>",
        json!({"f":[{"name":"Safha-archive Saudi voice.m4a","url":"https://velentstechnologiesinc-my.sharepoint.com/","type":"m4a"}]}),
    )?;
    msg(
        notes,
        None,
        ME,
        "Mohamed Eslam",
        3 * min,
        "<p>velents-docs-demo.mp4</p>",
        json!({"f":[{"name":"velents-docs-demo.mp4","url":"https://velentstechnologiesinc-my.sharepoint.com/","type":"mp4"}]}),
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
