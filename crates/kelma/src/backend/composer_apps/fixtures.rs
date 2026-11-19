//! Demo and test data for the composer's apps picker: synthetic answers in the shape the app
//! service and the bots send, parsed by the same functions as live answers. Nothing here comes
//! from a real tenant.

use app_store::bots::{QueryOutcome, parse_query_response};
use serde_json::{Value, json};

pub const GIF_APP: &str = "demo-gifs";
pub const TICKET_APP: &str = "demo-tickets";
pub const TASK_URL: &str = "https://tickets.example.invalid/kelma-demo-task";

/// The `chatentitlements` answer: two apps with compose commands, one whose only command is on
/// messages, and one that only has a bot.
pub fn place_apps() -> Value {
    json!({"value": {
        "entitlements": [
            {"id": GIF_APP, "state": "Installed"},
            {"id": TICKET_APP, "state": "Installed"},
            {"id": "demo-message-only", "state": "Installed"},
            {"id": "demo-bot-only", "state": "Installed"},
            {"id": "demo-not-installed", "state": "Featured"},
        ],
        "definitions": [
            {
                "id": GIF_APP,
                "name": "GIF Search",
                "inputExtensions": [{
                    "botId": "demo-gif-bot",
                    "scopes": ["personal", "team", "groupchat"],
                    "commands": [{
                        "id": "searchGifs",
                        "title": "Search GIFs",
                        "description": "Find a GIF to add to your message",
                        "context": ["compose", "commandbox"],
                        "parameters": [{"name": "query", "title": "Search", "inputType": "text"}],
                    }],
                }],
            },
            {
                "id": TICKET_APP,
                "name": "Contoso Tickets",
                "inputExtensions": [{
                    "botId": "demo-ticket-bot",
                    "scopes": ["team", "groupchat"],
                    "commands": [
                        {
                            "id": "findTicket",
                            "title": "Find a ticket",
                            "description": "Search the tickets",
                            "context": ["compose"],
                            "parameters": [{"name": "q", "title": "Ticket", "inputType": "text"}],
                        },
                        {
                            "id": "createTicket",
                            "title": "Create a ticket",
                            "description": "Opens the new ticket form",
                            "type": "action",
                            "context": ["compose"],
                            "taskInfo": {
                                "title": "Create a ticket",
                                "url": TASK_URL,
                                "width": "medium",
                                "height": "medium",
                            },
                        },
                        {
                            "id": "ticketFromMessage",
                            "title": "Ticket from message",
                            "type": "action",
                            "context": ["message"],
                            "taskInfo": {"title": "From a message", "url": TASK_URL},
                        },
                    ],
                }],
            },
            {
                "id": "demo-message-only",
                "name": "Message Tools",
                "inputExtensions": [{
                    "botId": "demo-message-bot",
                    "scopes": ["personal", "team", "groupchat"],
                    "commands": [{
                        "id": "translate",
                        "title": "Translate",
                        "type": "action",
                        "context": ["message"],
                        "fetchTask": true,
                    }],
                }],
            },
            {
                "id": "demo-bot-only",
                "name": "Helper Bot",
                "bots": [{"id": "demo-helper", "scopes": ["personal", "groupchat"]}],
            },
            {
                "id": "demo-not-installed",
                "name": "Not Installed",
                "inputExtensions": [{
                    "botId": "demo-x",
                    "scopes": ["personal", "team", "groupchat"],
                    "commands": [{"id": "x", "title": "X", "context": ["compose"]}],
                }],
            },
        ],
    }})
}

/// `(what the search matches, the attachment)`.
fn gifs() -> Vec<(&'static str, Value)> {
    let hero = |title: &str, subtitle: &str, url: &str| {
        json!({
            "contentType": "application/vnd.microsoft.card.hero",
            "content": {"title": title, "subtitle": subtitle, "images": [{"url": url}]},
        })
    };
    vec![
        (
            "cat typing",
            hero("Cat typing", "Cats", "https://gifs.example.invalid/cat.gif"),
        ),
        (
            "dog surfing",
            hero(
                "Dog surfing",
                "Dogs",
                "https://gifs.example.invalid/dog.gif",
            ),
        ),
        (
            "capybara bath",
            hero(
                "Capybara bath",
                "Capybaras",
                "https://gifs.example.invalid/capy.gif",
            ),
        ),
    ]
}

fn tickets() -> Vec<(&'static str, Value)> {
    let adaptive = |id: &str, title: &str, detail: &str| {
        json!({
            "contentType": "application/vnd.microsoft.card.adaptive",
            "content": {
                "type": "AdaptiveCard",
                "version": "1.4",
                "body": [
                    {"type": "TextBlock", "text": format!("{id} {title}"), "weight": "bolder"},
                    {"type": "TextBlock", "text": detail, "isSubtle": true},
                ],
            },
        })
    };
    vec![
        (
            "tck-101 login fails",
            adaptive("TCK-101", "Login fails", "Open, assigned to Sam"),
        ),
        (
            "tck-102 slow export",
            adaptive("TCK-102", "Slow export", "In progress, assigned to Dana"),
        ),
    ]
}

/// What the bot of `app_id` answers to the search command `command_id` with `text`.
pub fn query(app_id: &str, command_id: &str, text: &str) -> QueryOutcome {
    let all = match (app_id, command_id) {
        (GIF_APP, "searchGifs") => gifs(),
        (TICKET_APP, "findTicket") => tickets(),
        _ => Vec::new(),
    };
    let q = text.trim().to_lowercase();
    let hits: Vec<&Value> = all
        .iter()
        .filter(|(key, _)| q.is_empty() || key.contains(&q))
        .map(|(_, card)| card)
        .collect();
    parse_query_response(&json!({"composeExtension": {
        "type": "result",
        "attachmentLayout": "list",
        "attachments": hits,
    }}))
}
