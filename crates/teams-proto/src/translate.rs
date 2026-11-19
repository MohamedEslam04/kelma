//! Message translation (plans/chat.md 9.6).
//!
//! Teams translates a message by id: the client posts the ids to the chat service aggregator
//! (CSA) and gets the translated HTML back. The route, headers and answer shape come from the
//! Teams web client and rusty-teams (MIT, `chatsvc/src/translate.rs`, adapted here). There is no
//! call for free text: the middle-tier `beta/translate` of older clients answers 404 today.
//! Translating means sending the message to Microsoft's translator, so callers translate only
//! when the user asks, or when their translation setting says so.

use crate::client::Client;
use crate::error::{Error, Result};
use reqwest::Method;
use serde_json::{Value, json};

/// Most message ids the service takes in one call.
pub const MAX_BATCH: usize = 30;

/// Where a message lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Chat {
        chat_id: String,
    },
    /// A channel reply chain: `root_id` is the post the chain hangs from.
    Channel {
        team_id: String,
        channel_id: String,
        root_id: String,
    },
}

/// Why a translation is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// The user clicked "Translate".
    OnDemand,
    /// The user's setting translates on arrival; the service skips these languages.
    Automatic { known_languages: Vec<String> },
}

/// A message to translate. `version` pins an edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRef {
    pub id: String,
    pub version: Option<String>,
}

impl MessageRef {
    pub fn new(id: &str) -> Self {
        MessageRef {
            id: id.to_owned(),
            version: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Done,
    MessageNotFound,
    VersionNotFound,
    LanguageDetectionFailed,
    /// Empty, or already in a language the user knows.
    NotNeeded,
    Failed,
}

/// The translation of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageTranslation {
    pub message_id: String,
    pub version: Option<String>,
    pub status: Status,
    pub content_html: Option<String>,
    pub subject: Option<String>,
    pub title: Option<String>,
}

/// A language the translator supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// "Ask": show a Translate link.
    Suggest,
    /// "Yes": translate on arrival.
    Always,
    /// "No".
    Never,
}

/// The user's "translate to" setting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// The language of the Teams UI, which Teams translates to when no target is chosen.
    pub display_language: Option<String>,
    pub target_language: Option<String>,
    pub mode: Option<Mode>,
    /// Languages the user reads: messages in them are not offered for translation.
    pub known_languages: Vec<String>,
}

/// `languageStamp` of a message: the language Teams detected in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageStamp {
    pub code: String,
    pub score: u32,
}

/// Base of the chat service aggregator API.
const CSA_V1: &str = "https://teams.microsoft.com/api/csa/api/v1";

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Cuts a locale to the code the translator knows: `en-GB` is `en`, `zh-TW` is `zh-cht`.
pub fn language_code(locale: &str) -> String {
    let lowered = locale.trim().replace('_', "-").to_lowercase();
    let mut parts = lowered.split('-');
    let language = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    if language == "zh" {
        let traditional = rest
            .iter()
            .any(|p| matches!(*p, "hant" | "hk" | "tw" | "mo"));
        return if traditional { "zh-cht" } else { "zh-chs" }.to_owned();
    }
    match rest.first() {
        Some(script) if script.len() == 4 => format!("{language}-{script}"),
        _ => language.to_owned(),
    }
}

/// Path of the translate call under the CSA base (`…/api/v1`).
pub fn route(scope: &Scope, to: &str) -> String {
    match scope {
        Scope::Chat { chat_id } => {
            format!("chats/{}/messages/languages/{}", enc(chat_id), enc(to))
        }
        Scope::Channel {
            team_id,
            channel_id,
            root_id,
        } => format!(
            "teams/{}/channels/{}/{}/messages/languages/{}",
            enc(team_id),
            enc(channel_id),
            enc(root_id),
            enc(to)
        ),
    }
}

/// Headers of the translate call (besides auth). The Teams web client sends the same set.
pub fn headers(
    trigger: &Trigger,
    source_language: Option<&str>,
    request_id: &str,
) -> Vec<(String, String)> {
    let mut h = vec![
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("x-ms-request-id".to_owned(), request_id.to_owned()),
        ("x-ms-use-acs-translation".to_owned(), "true".to_owned()),
    ];
    match trigger {
        Trigger::OnDemand => h.push(("x-ms-on-demand-translation".to_owned(), "true".to_owned())),
        Trigger::Automatic { known_languages } => {
            h.push(("x-ms-knownLanguages".to_owned(), known_languages.join(",")))
        }
    }
    if let Some(l) = source_language {
        h.push(("x-ms-sourceLanguage".to_owned(), l.to_owned()));
    }
    h
}

pub fn body(messages: &[MessageRef]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|m| match &m.version {
                Some(v) => json!({"id": m.id, "version": v}),
                None => json!({"id": m.id}),
            })
            .collect(),
    )
}

fn status_for(code: &str) -> Status {
    match code {
        "Success" => Status::Done,
        "MessageNotFound" => Status::MessageNotFound,
        "VersionNotFound" => Status::VersionNotFound,
        "LanguageDetectionFailed" => Status::LanguageDetectionFailed,
        "SkipTranslation" | "ContentIsEmpty" => Status::NotNeeded,
        _ => Status::Failed,
    }
}

pub fn parse_message_translations(v: &Value) -> Result<Vec<MessageTranslation>> {
    let entries = v
        .as_array()
        .ok_or_else(|| Error::Protocol("translate: no result list".into()))?;
    Ok(entries
        .iter()
        .filter_map(|e| {
            let message_id = text(e, "id")?;
            Some(MessageTranslation {
                message_id,
                version: match e.get("version") {
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(Value::Number(n)) => Some(n.to_string()),
                    _ => None,
                },
                status: status_for(e.get("resultCode").and_then(Value::as_str).unwrap_or("")),
                content_html: text(e, "translatedContent"),
                subject: text(e, "translatedSubject"),
                title: text(e, "translatedTitle"),
            })
        })
        .collect())
}

pub fn parse_languages(v: &Value) -> Result<Vec<Language>> {
    let entries = v
        .as_array()
        .ok_or_else(|| Error::Protocol("translate: no language list".into()))?;
    Ok(entries
        .iter()
        .filter_map(|e| {
            Some(Language {
                code: text(e, "languageTag")?,
                name: text(e, "languageName")?,
            })
        })
        .collect())
}

/// Reads the settings from a `languageAccountSettings` object.
pub fn parse_settings(v: &Value) -> Settings {
    let locale = |x: &Value| text(x, "locale").map(|l| language_code(&l));
    Settings {
        display_language: v.get("defaultDisplayLanguage").and_then(locale),
        target_language: v.get("defaultTranslationLanguage").and_then(locale),
        mode: match v
            .pointer("/translationPreferences/translationBehavior")
            .and_then(Value::as_str)
        {
            Some("Yes") => Some(Mode::Always),
            Some("No") => Some(Mode::Never),
            Some("Ask") => Some(Mode::Suggest),
            _ => None,
        },
        known_languages: v
            .get("authoringLanguages")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(locale).collect())
            .unwrap_or_default(),
    }
}

/// `languages=de:100;length:82;&detector=Bling` is German, 100 percent.
pub fn parse_language_stamp(stamp: &str) -> Option<LanguageStamp> {
    let list = stamp.strip_prefix("languages=")?.split('&').next()?;
    let (code, score) = list.split(';').next()?.split_once(':')?;
    (!code.is_empty() && code != "null" && code != "length").then_some(())?;
    Some(LanguageStamp {
        code: code.to_owned(),
        score: score.parse().ok()?,
    })
}

impl Settings {
    /// The language to translate into: the chosen one, else the Teams UI language.
    pub fn target(&self) -> &str {
        self.target_language
            .as_deref()
            .or(self.display_language.as_deref())
            .unwrap_or("en")
    }

    /// What Teams does when the user never chose: suggest.
    pub fn effective_mode(&self) -> Mode {
        self.mode.unwrap_or(Mode::Suggest)
    }

    /// True when a message in `detected` should get a Translate link or be translated.
    pub fn should_translate(&self, detected: &str) -> bool {
        let detected = language_code(detected);
        self.effective_mode() != Mode::Never
            && self.target() != detected
            && !self.known_languages.contains(&detected)
    }
}

/// Translates the messages of one chat or channel chain into `to`, thirty ids per call. A
/// message the service could not translate comes back with a status other than
/// [`Status::Done`] rather than as an error.
pub async fn translate_messages(
    client: &Client,
    scope: &Scope,
    to: &str,
    messages: &[MessageRef],
    trigger: &Trigger,
) -> Result<Vec<MessageTranslation>> {
    let url = format!("{CSA_V1}/{}", route(scope, to));
    let mut out = Vec::new();
    for chunk in messages.chunks(MAX_BATCH) {
        let request_id = uuid::Uuid::new_v4().to_string();
        let all = headers(trigger, None, &request_id);
        // The JSON body sets its own content type.
        let extra: Vec<(&str, &str)> = all
            .iter()
            .filter(|(k, _)| k != "Content-Type")
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let (status, resp) = client
            .request(Method::POST, &url, Some(&body(chunk)), &extra)
            .await?;
        if !(200..300).contains(&status) {
            return Err(Error::status("translate", status, &resp));
        }
        out.extend(parse_message_translations(&serde_json::from_str(&resp)?)?);
    }
    Ok(out)
}

/// The languages the translator supports, named in `ui_locale`.
pub async fn languages(client: &Client, ui_locale: &str) -> Result<Vec<Language>> {
    let url = format!("{CSA_V1}/translator/languages?locale={}", enc(ui_locale));
    let (status, resp) = client
        .request(
            Method::GET,
            &url,
            None,
            &[("x-ms-use-acs-translation", "true")],
        )
        .await?;
    if !(200..300).contains(&status) {
        return Err(Error::status("translate_languages", status, &resp));
    }
    parse_languages(&serde_json::from_str(&resp)?)
}

/// Translates one message into `to`. A failure comes back as a status, not an error.
pub async fn translate_message(
    client: &Client,
    scope: &Scope,
    to: &str,
    message: &MessageRef,
    trigger: &Trigger,
) -> Result<MessageTranslation> {
    translate_messages(client, scope, to, std::slice::from_ref(message), trigger)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Protocol("translate: empty answer".into()))
}

/// The user's translation setting, read from the `userLanguageAccountSettings` aggregate.
pub async fn settings(client: &Client) -> Result<Settings> {
    let url = format!(
        "{}/beta/users/useraggregatesettings",
        client.middle_tier().await
    );
    let v = client
        .json(
            "translate_settings",
            Method::POST,
            &url,
            Some(&json!({"userLanguageAccountSettings": true})),
        )
        .await?;
    Ok(v.pointer("/userLanguageAccountSettings/value")
        .map(parse_settings)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
        h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn locales_map_to_translator_codes() {
        assert_eq!(language_code("en-GB"), "en");
        assert_eq!(language_code("de_DE"), "de");
        assert_eq!(language_code("ar-EG"), "ar");
        assert_eq!(language_code("zh-TW"), "zh-cht");
        assert_eq!(language_code("zh-Hant-HK"), "zh-cht");
        assert_eq!(language_code("zh-CN"), "zh-chs");
        assert_eq!(language_code("zh"), "zh-chs");
        assert_eq!(language_code("sr-Latn-RS"), "sr-latn");
        assert_eq!(language_code("FR"), "fr");
    }

    #[test]
    fn routes_for_chats_and_channel_chains_are_encoded() {
        let chat = Scope::Chat {
            chat_id: "19:a@thread.v2".into(),
        };
        assert_eq!(
            route(&chat, "en"),
            "chats/19%3Aa%40thread.v2/messages/languages/en"
        );
        let channel = Scope::Channel {
            team_id: "t1".into(),
            channel_id: "19:c@thread.tacv2".into(),
            root_id: "1700".into(),
        };
        assert_eq!(
            route(&channel, "ar"),
            "teams/t1/channels/19%3Ac%40thread.tacv2/1700/messages/languages/ar"
        );
    }

    #[test]
    fn on_demand_headers_force_and_omit_known_languages() {
        let h = headers(&Trigger::OnDemand, None, "rid");
        assert_eq!(find(&h, "x-ms-use-acs-translation"), Some("true"));
        assert_eq!(find(&h, "x-ms-on-demand-translation"), Some("true"));
        assert_eq!(find(&h, "x-ms-request-id"), Some("rid"));
        assert_eq!(find(&h, "Content-Type"), Some("application/json"));
        assert_eq!(find(&h, "x-ms-knownLanguages"), None);
        assert_eq!(find(&h, "x-ms-sourceLanguage"), None);
    }

    #[test]
    fn automatic_headers_list_known_languages_and_a_source_hint() {
        let h = headers(
            &Trigger::Automatic {
                known_languages: vec!["en".into(), "ar".into()],
            },
            Some("de"),
            "rid",
        );
        assert_eq!(find(&h, "x-ms-knownLanguages"), Some("en,ar"));
        assert_eq!(find(&h, "x-ms-on-demand-translation"), None);
        assert_eq!(find(&h, "x-ms-sourceLanguage"), Some("de"));
    }

    #[test]
    fn body_lists_ids_with_optional_versions() {
        let b = body(&[
            MessageRef::new("1"),
            MessageRef {
                id: "2".into(),
                version: Some("7".into()),
            },
        ]);
        assert_eq!(b, json!([{"id": "1"}, {"id": "2", "version": "7"}]));
    }

    #[test]
    fn parses_each_result_code() {
        let v = json!([
            {"id": "1", "version": 1700, "translatedContent": "<p>Hello</p>", "translatedSubject": null,
             "translatedTitle": "", "resultCode": "Success"},
            {"id": "2", "resultCode": "MessageNotFound"},
            {"id": "3", "resultCode": "VersionNotFound"},
            {"id": "4", "resultCode": "LanguageDetectionFailed"},
            {"id": "5", "resultCode": "SkipTranslation"},
            {"id": "6", "resultCode": "ContentIsEmpty"},
            {"id": "7", "resultCode": "Strange"},
            {"resultCode": "Success"},
        ]);
        let p = parse_message_translations(&v).unwrap();
        assert_eq!(p.len(), 7);
        assert_eq!(p[0].status, Status::Done);
        assert_eq!(p[0].version.as_deref(), Some("1700"));
        assert_eq!(p[0].content_html.as_deref(), Some("<p>Hello</p>"));
        assert_eq!(p[0].subject, None);
        assert_eq!(p[0].title, None);
        let statuses: Vec<_> = p[1..].iter().map(|t| t.status).collect();
        assert_eq!(
            statuses,
            [
                Status::MessageNotFound,
                Status::VersionNotFound,
                Status::LanguageDetectionFailed,
                Status::NotNeeded,
                Status::NotNeeded,
                Status::Failed
            ]
        );
        assert!(parse_message_translations(&json!("Bad request.")).is_err());
    }

    #[test]
    fn parses_the_language_list() {
        let l = parse_languages(&json!([
            {"languageTag": "ar", "languageName": "Arabic"},
            {"languageTag": "zh-chs", "languageName": "Chinese (Simplified)"},
            {"languageTag": "x"},
        ]))
        .unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].code, "ar");
        assert_eq!(l[1].name, "Chinese (Simplified)");
        assert!(parse_languages(&json!({})).is_err());
    }

    #[test]
    fn settings_map_the_account_settings_object() {
        let s = parse_settings(&json!({
            "defaultDisplayLanguage": {"locale": "ar-EG"},
            "defaultTranslationLanguage": {"locale": "en-US"},
            "translationPreferences": {"translationBehavior": "Ask"},
            "authoringLanguages": [{"locale": "ar-EG"}, {"locale": "en-US"}],
        }));
        assert_eq!(s.display_language.as_deref(), Some("ar"));
        assert_eq!(s.target_language.as_deref(), Some("en"));
        assert_eq!(s.mode, Some(Mode::Suggest));
        assert_eq!(s.known_languages, ["ar", "en"]);
        let s = parse_settings(&json!({"translationPreferences": {"translationBehavior": "Yes"}}));
        assert_eq!(s.mode, Some(Mode::Always));
        assert_eq!(s.target_language, None);
        let s = parse_settings(&json!({"translationPreferences": {"translationBehavior": "No"}}));
        assert_eq!(s.mode, Some(Mode::Never));
        assert_eq!(parse_settings(&json!({})), Settings::default());
        // Never chosen: the answer of an account that only has a display language.
        let s = parse_settings(
            &json!({"defaultDisplayLanguage": {"locale": "en-US"}, "authoringLanguages": []}),
        );
        assert_eq!((s.target(), s.effective_mode()), ("en", Mode::Suggest));
        assert!(s.should_translate("ar"));
        assert!(!s.should_translate("en"));
    }

    #[test]
    fn language_stamps_name_the_first_language() {
        let s = parse_language_stamp("languages=ar:98;en:2;length:82;&detector=Bling").unwrap();
        assert_eq!((s.code.as_str(), s.score), ("ar", 98));
        assert_eq!(parse_language_stamp("languages=null;&detector=Bling"), None);
        assert_eq!(parse_language_stamp("garbage"), None);
        assert_eq!(parse_language_stamp(""), None);
    }

    #[test]
    fn translation_is_offered_only_for_unknown_languages() {
        let s = Settings {
            display_language: None,
            target_language: Some("en".into()),
            mode: Some(Mode::Suggest),
            known_languages: vec!["en".into(), "ar".into()],
        };
        assert!(s.should_translate("de"));
        assert!(!s.should_translate("ar"));
        assert!(!s.should_translate("en"));
        let never = Settings {
            mode: Some(Mode::Never),
            ..s.clone()
        };
        assert!(!never.should_translate("de"));
        // No setting at all: Teams suggests translation, so a stranger language is offered.
        assert!(Settings::default().should_translate("de"));
    }
}
