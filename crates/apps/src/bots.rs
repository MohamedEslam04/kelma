//! Bots and messaging (compose) extensions installed in a chat or team, and the read-only
//! messaging-extension query.
//!
//! The app service lists the apps usable in a place: `GET beta/chats/<id>/apps/chatentitlements`
//! for a chat (also `48:notes`) and `GET beta/channels/<id>/apps/entitlements` for a team (its
//! General channel's thread id is the team id; `beta/teams/<id>/apps/entitlements` is POST
//! only). Both answer `value.entitlements` (`{id, state}`) with `value.definitions` (the app
//! manifests, verified live 2026-11-19). A definition's `bots` carry the bot's id, scopes and
//! `commandLists`; its `inputExtensions` (the manifest's `composeExtensions`) carry the
//! messaging extensions with their `commands`.
//!
//! A search-type ("query") command runs as a bot invoke the way the web client sends them: a
//! `POST` to `<chat service>/v1/agents/28:<botId>/invoke` whose body is the web client's invoke
//! envelope (`name`, `appId`, `value`, optional `conversation`) with `name`
//! `composeExtension/query` and the Bot Framework query value (`commandId`, `commandContext`,
//! `parameters`, `queryOptions`). Live (2026-11-19, a first-party search extension, no
//! `conversation`): 200 with `{"responseType":"composeExtension","composeExtension":{"type":
//! "silentAuth"}}`, so the route and envelope are accepted; `conversation` `48:notes` answers
//! 404 `ConversationNotFound` (bots are not members of the notes thread). The single sign-on
//! retry with `userAadToken` and result cards are built from the web client's response code
//! and the Bot Framework schema, not seen live.

use reqwest::Method;
use serde_json::{Value, json};
use teams_proto::graph::str_of;
use teams_proto::{Client, Error, Result};

use crate::bar::EntitlementState;
use crate::install::Request;
use crate::model::enc;

/// Where the apps are listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// A chat by its thread id (`19:…@thread.v2`, `19:…@unq.gbl.spaces`, `48:notes`).
    Chat(String),
    /// A team, by its thread id (`19:…@thread.tacv2`); a channel's thread id works too.
    Team(String),
}

/// The app service route that lists the apps of a place (relative to the middle tier).
pub fn entitlements_path(place: &Place) -> String {
    match place {
        Place::Chat(id) => format!("beta/chats/{}/apps/chatentitlements", enc(id)),
        Place::Team(id) => format!("beta/channels/{}/apps/entitlements", enc(id)),
    }
}

/// A slash command of a bot (`commandLists`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotCommand {
    pub title: String,
    pub description: String,
    /// `basic` (a plain command) or `prompt` (a suggested prompt).
    pub kind: String,
}

/// A bot of an app. It chats like a person (its MRI is `28:<id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bot {
    pub id: String,
    /// `Personal`, `Team`, `GroupChat`.
    pub scopes: Vec<String>,
    pub is_notification_only: bool,
    pub supports_files: bool,
    pub supports_calling: bool,
    pub supports_video: bool,
    pub commands: Vec<BotCommand>,
}

impl Bot {
    pub fn mri(&self) -> String {
        bot_mri(&self.id)
    }

    /// Works in the scope (`personal`, `team`, `groupchat`), case-insensitive.
    pub fn in_scope(&self, scope: &str) -> bool {
        has_scope(&self.scopes, scope)
    }
}

/// `28:<id>` for a bot id, unchanged when it already is an MRI.
pub fn bot_mri(id: &str) -> String {
    if id.starts_with("28:") {
        id.to_owned()
    } else {
        format!("28:{id}")
    }
}

fn has_scope(scopes: &[String], scope: &str) -> bool {
    scopes.iter().any(|s| s.eq_ignore_ascii_case(scope))
}

/// Search (`query`, a type-ahead list of results) or action (`action`, opens a task module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Query,
    Action,
}

/// Where a command shows: the compose box's extension flyout, the "/" command box, or a
/// message's "…" › More actions menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandContext {
    Compose,
    CommandBox,
    Message,
}

impl CommandContext {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "compose" => Some(Self::Compose),
            "commandbox" => Some(Self::CommandBox),
            "message" => Some(Self::Message),
            _ => None,
        }
    }

    /// The `commandContext` of the invoke value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compose => "compose",
            Self::CommandBox => "commandbox",
            Self::Message => "message",
        }
    }
}

/// How a command parameter is entered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Text,
    Textarea,
    Number,
    Date,
    Toggle,
    ChoiceSet,
    Other(String),
}

impl Input {
    fn parse(s: Option<&str>) -> Self {
        match s.map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("text") => Self::Text,
            Some("textarea") => Self::Textarea,
            Some("number") => Self::Number,
            Some("date") => Self::Date,
            Some("toggle") => Self::Toggle,
            Some("choiceset") => Self::ChoiceSet,
            Some(other) => Self::Other(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandParameter {
    pub name: String,
    pub title: String,
    pub description: String,
    pub required: bool,
    pub input: Input,
    /// `(title, value)` of a choice set.
    pub choices: Vec<(String, String)>,
}

/// The task module an action command opens when it has a static `taskInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInfo {
    pub title: String,
    pub url: String,
    /// `small`/`medium`/`large` or a pixel count, as the manifest wrote it.
    pub width: String,
    pub height: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionCommand {
    pub id: String,
    pub title: String,
    pub description: String,
    pub kind: CommandKind,
    pub contexts: Vec<CommandContext>,
    pub parameters: Vec<CommandParameter>,
    /// The bot is asked for the task module (`composeExtension/fetchTask`) when it runs.
    pub fetch_task: bool,
    /// A query runs once with no text when it opens.
    pub initial_run: bool,
    pub task: Option<TaskInfo>,
}

/// A messaging extension of an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub bot_id: String,
    pub scopes: Vec<String>,
    pub can_update_configuration: bool,
    /// The bot serves its commands at run time (`composeExtension/fetchCommands`).
    pub fetch_commands: bool,
    pub commands: Vec<ExtensionCommand>,
}

impl Extension {
    pub fn in_scope(&self, scope: &str) -> bool {
        has_scope(&self.scopes, scope)
    }
}

/// An installed app of a place that has a bot or a messaging extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceApp {
    pub id: String,
    pub name: String,
    pub small_icon: String,
    pub bots: Vec<Bot>,
    pub extensions: Vec<Extension>,
}

fn items<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v[key].as_array().into_iter().flatten()
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    items(v, key)
        .filter_map(|s| s.as_str().map(str::to_owned))
        .collect()
}

fn text(v: &Value, key: &str) -> String {
    str_of(v, key).unwrap_or_default()
}

/// A manifest size: `"medium"` or a number.
fn size(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn flag(v: &Value, key: &str) -> bool {
    v[key].as_bool().unwrap_or(false)
}

fn parse_bot(v: &Value) -> Option<Bot> {
    let commands = items(v, "commandLists")
        .flat_map(|l| items(l, "commands"))
        .map(|c| BotCommand {
            title: text(c, "title"),
            description: text(c, "description"),
            kind: str_of(c, "type").unwrap_or_else(|| "basic".into()),
        })
        .collect();
    Some(Bot {
        id: str_of(v, "id")?,
        scopes: strings(v, "scopes"),
        is_notification_only: flag(v, "isNotificationOnly"),
        supports_files: flag(v, "supportsFiles"),
        supports_calling: flag(v, "supportsCalling"),
        supports_video: flag(v, "supportsVideo"),
        commands,
    })
}

fn parse_parameter(v: &Value) -> Option<CommandParameter> {
    Some(CommandParameter {
        name: str_of(v, "name")?,
        title: text(v, "title"),
        description: text(v, "description"),
        required: flag(v, "isRequired"),
        input: Input::parse(v["inputType"].as_str()),
        choices: items(v, "choices")
            .map(|c| (text(c, "title"), text(c, "value")))
            .collect(),
    })
}

fn parse_command(v: &Value) -> Option<ExtensionCommand> {
    let mut contexts: Vec<CommandContext> = strings(v, "context")
        .iter()
        .filter_map(|c| CommandContext::parse(c))
        .collect();
    if contexts.is_empty() {
        // The manifest default.
        contexts = vec![CommandContext::Compose, CommandContext::CommandBox];
    }
    let task = v["taskInfo"]
        .as_object()
        .filter(|_| v["taskInfo"]["url"].as_str().is_some_and(|u| !u.is_empty()))
        .map(|_| TaskInfo {
            title: text(&v["taskInfo"], "title"),
            url: text(&v["taskInfo"], "url"),
            width: size(&v["taskInfo"]["width"]),
            height: size(&v["taskInfo"]["height"]),
        });
    Some(ExtensionCommand {
        id: str_of(v, "id")?,
        title: text(v, "title"),
        description: text(v, "description"),
        kind: match v["type"].as_str() {
            Some(t) if t.eq_ignore_ascii_case("action") => CommandKind::Action,
            _ => CommandKind::Query,
        },
        contexts,
        parameters: items(v, "parameters").filter_map(parse_parameter).collect(),
        fetch_task: flag(v, "fetchTask"),
        initial_run: flag(v, "initialRun"),
        task,
    })
}

fn parse_extension(v: &Value) -> Option<Extension> {
    Some(Extension {
        bot_id: str_of(v, "botId")?,
        scopes: strings(v, "scopes"),
        can_update_configuration: flag(v, "canUpdateConfiguration"),
        fetch_commands: flag(v, "fetchCommands"),
        commands: items(v, "commands").filter_map(parse_command).collect(),
    })
}

/// The installed apps of the answer that have a bot or a messaging extension. When the answer
/// carries entitlements, an app is listed only when its entitlement is installed; without
/// entitlements every definition counts.
pub fn parse_place_apps(v: &Value) -> Vec<PlaceApp> {
    let value = &v["value"];
    let states: Option<Vec<(String, EntitlementState)>> =
        value["entitlements"].as_array().map(|list| {
            list.iter()
                .filter_map(|e| {
                    Some((
                        str_of(e, "id")?,
                        EntitlementState::parse(e["state"].as_str().unwrap_or_default()),
                    ))
                })
                .collect()
        });
    items(value, "definitions")
        .filter_map(|d| {
            let id = str_of(d, "id")?;
            if let Some(states) = &states
                && !states.iter().any(|(i, s)| *i == id && s.is_installed())
            {
                return None;
            }
            let bots: Vec<Bot> = items(d, "bots").filter_map(parse_bot).collect();
            let extensions: Vec<Extension> = items(d, "inputExtensions")
                .filter_map(parse_extension)
                .collect();
            if bots.is_empty() && extensions.is_empty() {
                return None;
            }
            Some(PlaceApp {
                name: text(d, "name"),
                small_icon: text(d, "smallImageUrl"),
                id,
                bots,
                extensions,
            })
        })
        .collect()
}

/// The bots and messaging extensions installed in a chat or team.
pub async fn place_apps(client: &Client, place: &Place) -> Result<Vec<PlaceApp>> {
    let v = crate::api::read(
        client,
        "place apps",
        Method::GET,
        &entitlements_path(place),
        None,
    )
    .await?;
    Ok(parse_place_apps(&v))
}

/// One run of a search-type command.
pub struct QueryRequest<'a> {
    pub app_id: &'a str,
    /// The extension's bot id (or MRI).
    pub bot_id: &'a str,
    pub command_id: &'a str,
    pub context: CommandContext,
    /// `(parameter name, value)`.
    pub parameters: &'a [(String, String)],
    pub skip: u32,
    pub count: u32,
    /// The chat or channel the user is in; left out when empty (the web client leaves it out
    /// without a thread).
    pub conversation_id: &'a str,
    /// The user's display name; left out when empty.
    pub display_name: &'a str,
}

/// The `composeExtension/query` bot invoke. `chat_service` is the chat service base URL.
pub fn query_request(chat_service: &str, q: &QueryRequest<'_>) -> Request {
    let parameters: Vec<Value> = q
        .parameters
        .iter()
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect();
    let mut body = json!({
        "name": "composeExtension/query",
        "appId": q.app_id,
        "value": {
            "commandId": q.command_id,
            "commandContext": q.context.as_str(),
            "parameters": parameters,
            "queryOptions": {"skip": q.skip, "count": q.count},
        },
    });
    if !q.conversation_id.is_empty() {
        body["conversation"] = json!({"id": q.conversation_id});
    }
    if !q.display_name.is_empty() {
        body["imdisplayname"] = json!(q.display_name);
    }
    Request {
        method: Method::POST,
        url: format!(
            "{}/v1/agents/{}/invoke",
            chat_service.trim_end_matches('/'),
            bot_mri(q.bot_id)
        ),
        body: Some(body),
    }
}

/// What a query answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultKind {
    /// Result cards to pick from.
    Result,
    /// The user has to sign in first (`actions` holds the sign-in link).
    Auth,
    /// The bot wants single sign-on first: the client gets an AAD token for the app
    /// (`webApplicationInfo`) and asks again with it as `userAadToken` (seen live).
    SilentAuth,
    /// The extension needs configuring (`actions` holds the settings link).
    Config,
    /// A plain message (nothing found, an error text).
    Message,
    /// A result to preview before sending.
    BotMessagePreview,
    Other,
}

/// One result of a query: the card to send and the preview the result list shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultItem {
    pub content_type: String,
    pub content: Value,
    pub title: String,
    pub subtitle: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryResults {
    pub kind: ResultKind,
    /// `list` or `grid`.
    pub layout: Option<String>,
    pub text: Option<String>,
    pub items: Vec<ResultItem>,
    /// `(title, value)` of the suggested actions (the sign-in or settings link).
    pub actions: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryOutcome {
    Results(QueryResults),
    /// A task module or another answer a query does not give.
    Other,
    Empty,
    Failed {
        status: u16,
        message: String,
    },
}

/// An attachment's card, which the bot may send as an object or as a JSON string.
fn card(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| v.clone()),
        other => other.clone(),
    }
}

fn parse_item(a: &Value) -> ResultItem {
    let content = card(&a["content"]);
    let preview = card(&a["preview"]["content"]);
    let pick = |keys: &[&str]| -> String {
        [&preview, &content]
            .into_iter()
            .flat_map(|c| keys.iter().map(move |k| c[*k].as_str()))
            .flatten()
            .find(|s| !s.is_empty())
            .unwrap_or_default()
            .to_owned()
    };
    let image = [&preview, &content]
        .into_iter()
        .find_map(|c| c["images"][0]["url"].as_str().filter(|u| !u.is_empty()))
        .map(str::to_owned);
    ResultItem {
        content_type: text(a, "contentType"),
        title: pick(&["title"]),
        subtitle: pick(&["subtitle", "text"]),
        image,
        content,
    }
}

/// Parses the invoke answer of a query.
pub fn parse_query_response(v: &Value) -> QueryOutcome {
    if v.is_null() || v.as_object().is_some_and(|m| m.is_empty()) {
        return QueryOutcome::Empty;
    }
    if let Some(status) = v["statusCode"].as_u64().filter(|s| *s >= 400) {
        return QueryOutcome::Failed {
            status: status as u16,
            message: match &v["value"] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            },
        };
    }
    let ext = &v["composeExtension"];
    if !ext.is_object() {
        return QueryOutcome::Other;
    }
    QueryOutcome::Results(QueryResults {
        kind: match ext["type"].as_str() {
            Some("result") => ResultKind::Result,
            Some("auth") => ResultKind::Auth,
            Some("silentAuth") => ResultKind::SilentAuth,
            Some("config") => ResultKind::Config,
            Some("message") => ResultKind::Message,
            Some("botMessagePreview") => ResultKind::BotMessagePreview,
            _ => ResultKind::Other,
        },
        layout: str_of(ext, "attachmentLayout"),
        text: str_of(ext, "text"),
        items: items(ext, "attachments").map(parse_item).collect(),
        actions: items(&ext["suggestedActions"], "actions")
            .filter_map(|a| Some((text(a, "title"), str_of(a, "value")?)))
            .collect(),
    })
}

/// Runs a search-type command of a messaging extension and returns its results. Read only: it
/// answers results to pick from and sends no message. An action command opens a task module
/// and can post, so it is refused here.
#[allow(clippy::too_many_arguments)]
pub async fn query_extension(
    client: &Client,
    app: &PlaceApp,
    extension: &Extension,
    command: &ExtensionCommand,
    context: CommandContext,
    parameters: &[(String, String)],
    conversation_id: &str,
    display_name: &str,
) -> Result<QueryOutcome> {
    if command.kind != CommandKind::Query {
        return Err(Error::Protocol(format!(
            "command {} is an action, not a query",
            command.id
        )));
    }
    let req = query_request(
        &client.chat_service().await,
        &QueryRequest {
            app_id: &app.id,
            bot_id: &extension.bot_id,
            command_id: &command.id,
            context,
            parameters,
            skip: 0,
            count: 25,
            conversation_id,
            display_name,
        },
    );
    let (status, body) = client
        .request(req.method, &req.url, req.body.as_ref(), &[])
        .await?;
    if !(200..300).contains(&status) {
        return Err(Error::status("messaging extension query", status, &body));
    }
    if body.trim().is_empty() {
        return Ok(QueryOutcome::Empty);
    }
    Ok(parse_query_response(&serde_json::from_str(&body)?))
}
