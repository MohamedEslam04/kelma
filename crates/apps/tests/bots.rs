//! Bots and messaging extensions of a chat or team, against synthetic fixtures shaped like
//! the live answers (2026-11-19).

use apps::bots::{
    CommandContext, CommandKind, Input, Place, QueryOutcome, QueryRequest, ResultKind,
    entitlements_path, parse_place_apps, parse_query_response, query_request,
};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn entitlement_routes_per_place() {
    assert_eq!(
        entitlements_path(&Place::Chat("19:abc@thread.v2".into())),
        "beta/chats/19%3Aabc%40thread.v2/apps/chatentitlements"
    );
    assert_eq!(
        entitlements_path(&Place::Chat("48:notes".into())),
        "beta/chats/48%3Anotes/apps/chatentitlements"
    );
    // A team is read through its channel route (the General channel's thread id is the team id).
    assert_eq!(
        entitlements_path(&Place::Team("19:abc@thread.tacv2".into())),
        "beta/channels/19%3Aabc%40thread.tacv2/apps/entitlements"
    );
}

#[test]
fn only_installed_apps_with_a_bot_or_extension_are_listed() {
    let apps = parse_place_apps(&fixture("chat_entitlements.json"));
    let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
    // Tabs-only, not installed and id-less definitions stay out.
    assert_eq!(names, ["Contoso Tickets", "Contoso Wiki"]);
    let tickets = &apps[0];
    assert_eq!(tickets.id, "11111111-aaaa-bbbb-cccc-000000000001");
    assert_eq!(tickets.small_icon, "https://cdn.example/tickets-small.png");
}

#[test]
fn bots_keep_their_commands_and_flags() {
    let apps = parse_place_apps(&fixture("chat_entitlements.json"));
    let bots = &apps[0].bots;
    assert_eq!(bots.len(), 2);
    let bot = &bots[0];
    assert_eq!(bot.id, "aaaaaaaa-0000-0000-0000-000000000001");
    assert_eq!(bot.mri(), "28:aaaaaaaa-0000-0000-0000-000000000001");
    assert!(bot.supports_files && !bot.is_notification_only);
    assert!(bot.in_scope("personal") && bot.in_scope("Team") && !bot.in_scope("groupchat"));
    let titles: Vec<(&str, &str)> = bot
        .commands
        .iter()
        .map(|c| (c.title.as_str(), c.kind.as_str()))
        .collect();
    assert_eq!(
        titles,
        [
            ("help", "basic"),
            ("summarize", "prompt"),
            ("legacy", "basic")
        ]
    );
    assert!(bots[1].is_notification_only && bots[1].commands.is_empty());
}

#[test]
fn extensions_expose_commands_with_type_context_and_parameters() {
    let apps = parse_place_apps(&fixture("chat_entitlements.json"));
    let ext = &apps[0].extensions[0];
    assert_eq!(ext.bot_id, "aaaaaaaa-0000-0000-0000-000000000001");
    assert!(ext.can_update_configuration && !ext.fetch_commands);
    assert!(ext.in_scope("groupchat") && ext.in_scope("TEAM"));
    assert_eq!(ext.commands.len(), 3);

    let search = &ext.commands[0];
    assert_eq!(search.id, "search");
    assert_eq!(search.kind, CommandKind::Query);
    assert_eq!(
        search.contexts,
        [CommandContext::Compose, CommandContext::CommandBox]
    );
    assert!(search.initial_run && !search.fetch_task && search.task.is_none());
    assert_eq!(search.parameters.len(), 2);
    let keyword = &search.parameters[0];
    assert_eq!(keyword.name, "keyword");
    assert!(keyword.required);
    assert_eq!(keyword.input, Input::Text);
    let status = &search.parameters[1];
    assert_eq!(status.input, Input::ChoiceSet);
    assert!(!status.required);
    assert_eq!(
        status.choices,
        [
            ("Open".into(), "open".into()),
            ("Closed".into(), "closed".into())
        ]
    );

    let create = &ext.commands[1];
    assert_eq!(create.kind, CommandKind::Action);
    assert_eq!(
        create.contexts,
        [CommandContext::Message, CommandContext::Compose]
    );
    assert!(create.fetch_task);
    let task = create.task.as_ref().unwrap();
    assert_eq!(task.url, "https://tickets.example/new");
    assert_eq!(
        (task.width.as_str(), task.height.as_str()),
        ("medium", "large")
    );
    assert_eq!(create.parameters[0].input, Input::Textarea);

    // A command without a type is a query; without a context the manifest default applies
    // (the compose box and the command box).
    let defaults = &ext.commands[2];
    assert_eq!(defaults.kind, CommandKind::Query);
    assert_eq!(
        defaults.contexts,
        [CommandContext::Compose, CommandContext::CommandBox]
    );
}

#[test]
fn dynamic_extensions_have_no_static_commands() {
    let apps = parse_place_apps(&fixture("chat_entitlements.json"));
    let ext = &apps[1].extensions[0];
    assert!(ext.fetch_commands && ext.commands.is_empty());
    assert!(apps[1].bots.is_empty());
}

#[test]
fn answers_without_entitlements_list_every_definition() {
    // Without an `entitlements` list there is nothing to filter by.
    let v = json!({"value": {"definitions": [
        {"id": "a", "name": "A", "bots": [{"id": "b1"}]}
    ]}});
    let apps = parse_place_apps(&v);
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].bots[0].id, "b1");
    assert!(parse_place_apps(&Value::Null).is_empty());
}

#[test]
fn query_request_is_a_bot_invoke_with_the_command_and_parameters() {
    let req = query_request(
        "https://teams.example/api/chatsvc/emea/",
        &QueryRequest {
            app_id: "11111111-aaaa-bbbb-cccc-000000000001",
            bot_id: "aaaaaaaa-0000-0000-0000-000000000001",
            command_id: "search",
            context: CommandContext::CommandBox,
            parameters: &[("keyword".into(), "printer".into())],
            skip: 0,
            count: 25,
            conversation_id: "48:notes",
            display_name: "",
        },
    );
    assert_eq!(req.method, reqwest::Method::POST);
    assert_eq!(
        req.url,
        "https://teams.example/api/chatsvc/emea/v1/agents/28:aaaaaaaa-0000-0000-0000-000000000001/invoke"
    );
    assert_eq!(
        req.body.unwrap(),
        json!({
            "name": "composeExtension/query",
            "appId": "11111111-aaaa-bbbb-cccc-000000000001",
            "value": {
                "commandId": "search",
                "commandContext": "commandbox",
                "parameters": [{"name": "keyword", "value": "printer"}],
                "queryOptions": {"skip": 0, "count": 25}
            },
            "conversation": {"id": "48:notes"}
        })
    );
}

#[test]
fn query_request_accepts_a_bot_mri_and_a_display_name() {
    let req = query_request(
        "https://teams.example/api/chatsvc/emea",
        &QueryRequest {
            app_id: "app",
            bot_id: "28:abc",
            command_id: "c",
            context: CommandContext::Compose,
            parameters: &[],
            skip: 10,
            count: 5,
            conversation_id: "19:x@thread.v2",
            display_name: "Test User",
        },
    );
    assert!(req.url.ends_with("/v1/agents/28:abc/invoke"));
    let body = req.body.unwrap();
    assert_eq!(body["imdisplayname"], "Test User");
    assert_eq!(body["value"]["commandContext"], "compose");
    assert_eq!(
        body["value"]["queryOptions"],
        json!({"skip": 10, "count": 5})
    );
}

#[test]
fn query_answer_gives_the_items_with_their_previews() {
    let QueryOutcome::Results(r) = parse_query_response(&fixture("compose_query_answer.json"))
    else {
        panic!("expected results");
    };
    assert_eq!(r.kind, ResultKind::Result);
    assert_eq!(r.layout.as_deref(), Some("list"));
    assert_eq!(r.items.len(), 2);
    let first = &r.items[0];
    assert_eq!(
        first.content_type,
        "application/vnd.microsoft.card.thumbnail"
    );
    // The preview card is what the result list shows.
    assert_eq!(first.title, "TCK-1");
    assert_eq!(first.subtitle, "Printer on fire");
    assert_eq!(
        first.image.as_deref(),
        Some("https://cdn.example/p-small.png")
    );
    assert_eq!(first.content["title"], "TCK-1 Printer on fire");
    // An adaptive card sent as a JSON string is parsed.
    let second = &r.items[1];
    assert_eq!(second.content["type"], "AdaptiveCard");
    assert_eq!(second.title, "");
}

#[test]
fn query_answer_variants() {
    let auth = json!({"responseType": "composeExtension", "composeExtension": {
        "type": "auth",
        "suggestedActions": {"actions": [{"type": "openUrl", "title": "Sign in", "value": "https://login.example/x"}]}
    }});
    let QueryOutcome::Results(r) = parse_query_response(&auth) else {
        panic!()
    };
    assert_eq!(r.kind, ResultKind::Auth);
    assert_eq!(
        r.actions,
        [("Sign in".into(), "https://login.example/x".into())]
    );
    assert!(r.items.is_empty());

    // Seen live from a first-party search extension.
    let sso =
        json!({"composeExtension": {"type": "silentAuth"}, "responseType": "composeExtension"});
    let QueryOutcome::Results(r) = parse_query_response(&sso) else {
        panic!()
    };
    assert_eq!(r.kind, ResultKind::SilentAuth);

    let message = json!({"responseType": "composeExtension", "composeExtension": {"type": "message", "text": "Nothing found"}});
    let QueryOutcome::Results(r) = parse_query_response(&message) else {
        panic!()
    };
    assert_eq!(r.kind, ResultKind::Message);
    assert_eq!(r.text.as_deref(), Some("Nothing found"));

    // A task answer is not a query result; an empty answer is empty.
    let task = json!({"task": {"type": "continue", "value": {"url": "https://x"}}});
    assert!(matches!(parse_query_response(&task), QueryOutcome::Other));
    assert!(matches!(
        parse_query_response(&Value::Null),
        QueryOutcome::Empty
    ));
    let failed = json!({"statusCode": 502, "value": "Bad gateway"});
    assert!(matches!(
        parse_query_response(&failed),
        QueryOutcome::Failed { status: 502, .. }
    ));
}

#[test]
fn query_request_leaves_out_an_empty_conversation() {
    let req = query_request(
        "https://teams.example/api/chatsvc/emea",
        &QueryRequest {
            app_id: "app",
            bot_id: "bot",
            command_id: "c",
            context: CommandContext::Compose,
            parameters: &[],
            skip: 0,
            count: 25,
            conversation_id: "",
            display_name: "",
        },
    );
    let body = req.body.unwrap();
    assert!(body.get("conversation").is_none() && body.get("imdisplayname").is_none());
}
