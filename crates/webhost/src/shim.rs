//! Host side of the TeamsJS protocol (`@microsoft/teams-js` 2.x).
//!
//! A web host window is a top-level page, so teams-js finds no parent window and falls back to
//! its "frameless" transport: it calls `window.nativeInterface.framelessPostMessage(json)` and
//! expects replies through `window.onNativeMessage({ data })`. The injected script
//! ([`SHIM_JS`]) forwards those strings to Rust over a CDP binding, and [`handle`] turns each
//! request into a reply, an [`AppEvent`] and, for `authentication.getAuthToken`, a
//! [`TokenRequest`] for the caller's token callback. Everything here is pure (no browser).
//!
//! Wire format (teams-js `communication.js`, `messageObjects.js`):
//! - request: `{ id, uuidAsString, func, args, apiVersionTag, timestamp, ... }`
//! - response: `{ id, uuidAsString, args }` (matched by `uuidAsString`, else by `id`)
//! - host event: `{ func, args }`, for example `{ func: "themeChange", args: ["dark"] }`

use serde_json::{Value, json};

/// Name of the CDP binding the shim posts teams-js messages to.
pub const TEAMS_BINDING: &str = "__kelmaTeams";
/// Name of the CDP binding behind `window.kelma.invoke`.
pub const BRIDGE_BINDING: &str = "__kelmaBridge";

/// Injected into every document before its own scripts run. It defines `window.kelma.invoke`
/// (the generic bridge) and, when `window.__kelmaShim` is set first, `window.nativeInterface`.
pub const SHIM_JS: &str = include_str!("shim.js");

/// The `clientSupportedSDKVersion` the host reports in the `initialize` reply.
pub const HOST_SDK_VERSION: &str = "2.59.0";

/// teams-js `ErrorCode.NOT_SUPPORTED_ON_PLATFORM`.
const NOT_SUPPORTED: i64 = 100;

/// What `getContext` reports. Field names follow the teams-js legacy `Context`.
#[derive(Clone, Debug)]
pub struct HostContext {
    pub app_id: String,
    pub entity_id: String,
    pub sub_entity_id: Option<String>,
    pub user_object_id: String,
    pub user_principal_name: String,
    pub user_display_name: String,
    pub tenant_id: String,
    /// `default`, `dark` or `contrast`.
    pub theme: String,
    /// BCP 47, for example `en-us` or `ar-sa`.
    pub locale: String,
    pub app_session_id: String,
    /// Reported as `app.host.name`. Apps commonly check for `Teams`.
    pub host_name: String,
}

impl Default for HostContext {
    fn default() -> Self {
        HostContext {
            app_id: String::new(),
            entity_id: String::new(),
            sub_entity_id: None,
            user_object_id: String::new(),
            user_principal_name: String::new(),
            user_display_name: String::new(),
            tenant_id: String::new(),
            theme: "default".into(),
            locale: "en-us".into(),
            app_session_id: uuid::Uuid::new_v4().to_string(),
            host_name: "Teams".into(),
        }
    }
}

/// Shim settings for one window.
#[derive(Clone, Debug, Default)]
pub struct ShimConfig {
    pub context: HostContext,
    /// Origins (`https://app.example.com`) whose pages may talk to the host. Messages from any
    /// other origin are dropped, so a page the app navigates to cannot ask for tokens. Empty
    /// means the origin of the URL passed to `open`.
    pub allowed_origins: Vec<String>,
    /// The app's `webApplicationInfo.resource` (`api://<domain>/<app id>`). When set, it is the
    /// only resource `getAuthToken` serves, as in Teams; when not, the one resource the app
    /// asks for. Either way it must be on the page's own domain, see [`check_token_request`].
    pub sso_resource: Option<String>,
    /// The manifest's `validDomains` (`contoso.com`, `*.contoso.com`, `localhost:3000`): other
    /// domains an app resource may name besides the page's own host.
    pub valid_domains: Vec<String>,
}

impl ShimConfig {
    /// The config for one app tab: `tab_url` is the tab's (already expanded) URL. Shim calls are
    /// accepted from the tab's origin and from the exact https hosts the manifest's
    /// `valid_domains` name. A wildcard entry (`*.contoso.com`) names no single origin, so it
    /// only widens the app resource check ([`check_token_request`]), never who may talk to the
    /// host.
    pub fn for_app(
        context: HostContext,
        tab_url: &str,
        sso_resource: Option<String>,
        valid_domains: Vec<String>,
    ) -> ShimConfig {
        let mut allowed_origins: Vec<String> = origin_of(tab_url).into_iter().collect();
        for d in &valid_domains {
            let plain = !d.is_empty()
                && d.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
            if let Some(o) = plain.then(|| origin_of(&format!("https://{d}"))).flatten()
                && !allowed_origins.iter().any(|a| same_origin(a, &o))
            {
                allowed_origins.push(o);
            }
        }
        ShimConfig {
            context,
            allowed_origins,
            sso_resource,
            valid_domains,
        }
    }
}

/// The origin (`https://host[:port]`) of an http(s) URL, or `None` for anything else.
pub fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    matches!(u.scheme(), "http" | "https").then(|| u.origin().ascii_serialization())
}

/// An `authentication.getAuthToken` call, for the caller's token callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRequest {
    pub resources: Vec<String>,
    pub claims: Vec<String>,
    pub silent: bool,
    pub tenant_id: Option<String>,
    /// Origin of the page that asked (taken from the CDP execution context, not the page).
    pub origin: String,
}

/// Where a reply goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplyTo {
    pub id: i64,
    pub uuid: Option<String>,
}

/// App lifecycle and requests the UI may act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEvent {
    /// `app.notifyAppLoaded`.
    Loaded,
    /// `app.notifySuccess`: hide the loading indicator.
    Success,
    /// `app.notifyFailure`.
    Failure { reason: String, message: String },
    /// `app.notifyExpectedFailure`.
    ExpectedFailure { reason: String, message: String },
    /// `app.openLink` / `executeDeepLink`.
    OpenLink(String),
    /// A call the shim does not implement (answered with "not supported").
    Unsupported(String),
}

/// Result of handling one message from the page.
#[derive(Debug, Default, PartialEq)]
pub struct Handled {
    /// Response to deliver now with [`deliver_js`].
    pub reply: Option<Value>,
    pub event: Option<AppEvent>,
    /// A token to fetch; answer it with [`token_response`].
    pub token: Option<(TokenRequest, ReplyTo)>,
}

/// Handles one `framelessPostMessage` string sent from a page of `origin`.
pub fn handle(cfg: &ShimConfig, raw: &str, origin: &str) -> Handled {
    if !cfg.allowed_origins.iter().any(|o| same_origin(o, origin)) {
        tracing::debug!(origin, "teams-js message from a disallowed origin dropped");
        return Handled::default();
    }
    let Ok(msg) = serde_json::from_str::<Value>(raw) else {
        return Handled::default();
    };
    let (Some(id), Some(func)) = (msg["id"].as_i64(), msg["func"].as_str()) else {
        return Handled::default();
    };
    let to = ReplyTo {
        id,
        uuid: msg["uuidAsString"].as_str().map(str::to_string),
    };
    let args = msg["args"].as_array().cloned().unwrap_or_default();
    let arg = |i: usize| {
        args.get(i)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let reply = |args: Value| Handled {
        reply: Some(response(&to, args)),
        ..Handled::default()
    };
    let event = |event: AppEvent| Handled {
        event: Some(event),
        ..Handled::default()
    };
    match func {
        "initialize" => reply(json!([
            "content",
            "desktop",
            runtime_config().to_string(),
            HOST_SDK_VERSION
        ])),
        "getContext" => reply(json!([legacy_context(&cfg.context)])),
        "authentication.getAuthToken" => {
            let strings = |v: Option<&Value>| -> Vec<String> {
                v.and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let request = TokenRequest {
                resources: strings(args.first()),
                claims: strings(args.get(1)),
                silent: args.get(2).and_then(Value::as_bool).unwrap_or(false),
                tenant_id: args.get(3).and_then(Value::as_str).map(str::to_string),
                origin: origin.to_string(),
            };
            Handled {
                token: Some((request, to)),
                ..Handled::default()
            }
        }
        "appInitialization.appLoaded" => event(AppEvent::Loaded),
        "appInitialization.success" => event(AppEvent::Success),
        "appInitialization.failure" => event(AppEvent::Failure {
            reason: arg(0),
            message: arg(1),
        }),
        "appInitialization.expectedFailure" => event(AppEvent::ExpectedFailure {
            reason: arg(0),
            message: arg(1),
        }),
        // One-way messages: handler registrations and the unload handshake.
        "registerHandler" | "readyToUnload" => Handled::default(),
        "executeDeepLink" => Handled {
            event: Some(AppEvent::OpenLink(arg(0))),
            ..reply(json!([true]))
        },
        other => Handled {
            event: Some(AppEvent::Unsupported(other.to_string())),
            ..reply(json!([{
                "errorCode": NOT_SUPPORTED,
                "message": format!("{other} is not supported by kelma"),
            }]))
        },
    }
}

fn same_origin(a: &str, b: &str) -> bool {
    a.trim_end_matches('/')
        .eq_ignore_ascii_case(b.trim_end_matches('/'))
}

fn response(to: &ReplyTo, args: Value) -> Value {
    let mut r = json!({ "id": to.id, "args": args });
    if let Some(uuid) = &to.uuid {
        r["uuidAsString"] = json!(uuid);
    }
    r
}

/// The runtime the host reports: only what the shim answers, so teams-js' own `isSupported()`
/// checks keep apps from calling the rest.
fn runtime_config() -> Value {
    json!({
        "apiVersion": 4,
        "hostVersionsInfo": {
            "adaptiveCardSchemaVersion": { "majorVersion": 1, "minorVersion": 5 }
        },
        "isLegacyTeams": false,
        "supports": { "authentication": {} },
    })
}

fn legacy_context(c: &HostContext) -> Value {
    json!({
        "frameContext": "content",
        "hostClientType": "desktop",
        "hostName": c.host_name,
        "appId": c.app_id,
        "entityId": c.entity_id,
        "subEntityId": c.sub_entity_id,
        "userObjectId": c.user_object_id,
        "userPrincipalName": c.user_principal_name,
        "loginHint": c.user_principal_name,
        "userDisplayName": c.user_display_name,
        "tid": c.tenant_id,
        "theme": c.theme,
        "locale": c.locale,
        "appSessionId": c.app_session_id,
        "isFullScreen": false,
        "isMultiWindow": true,
    })
}

/// Checks a `getAuthToken` request before anything is minted and returns it with `resources`
/// set to the one resource to mint. An app token may only go to the app's own origin:
///
/// - one resource per request: the manifest's [`ShimConfig::sso_resource`] when known (an app
///   may then only ask for that one), else the single resource the app asked for;
/// - it has the tab SSO form `api://<domain>/<app id>` (or `/botid-<id>`); forms without a domain
///   (`api://<app id>`, `api://botid-<id>`) cannot be tied to a page, and first-party resources
///   (`https://…`) are never handed to an app;
/// - the page is https and `<domain>` (with its port, 443 when none) is the page's host and
///   port, or one of [`ShimConfig::valid_domains`];
/// - a requested tenant must be the signed-in one.
///
/// The `Err` is the reason given to the app.
pub fn check_token_request(
    cfg: &ShimConfig,
    mut req: TokenRequest,
) -> Result<TokenRequest, String> {
    let tenant = &cfg.context.tenant_id;
    if let Some(t) = req.tenant_id.as_deref().filter(|t| !t.is_empty())
        && !tenant.is_empty()
        && !t.eq_ignore_ascii_case(tenant)
    {
        return Err(format!("kelma cannot get tokens for tenant {t}"));
    }
    let resource = match (&cfg.sso_resource, req.resources.as_slice()) {
        (Some(own), []) => own.clone(),
        (Some(own), [asked]) if asked == own => own.clone(),
        (Some(own), _) => return Err(format!("this app can only get a token for {own}")),
        (None, [asked]) => asked.clone(),
        (None, []) => return Err("no resource: the app has no webApplicationInfo.resource".into()),
        (None, _) => return Err("kelma gets a token for one resource at a time".into()),
    };
    let (host, port) = app_resource_authority(&resource)?;
    let page = url::Url::parse(&req.origin)
        .ok()
        .filter(|u| u.scheme() == "https")
        .ok_or_else(|| {
            format!(
                "app tokens are only given to https pages, not {}",
                req.origin
            )
        })?;
    let same_page = page.host_str().map(str::to_ascii_lowercase).as_deref() == Some(host.as_str())
        && page.port_or_known_default() == Some(port.unwrap_or(443));
    let authority = match port {
        Some(p) => format!("{host}:{p}"),
        None => host.clone(),
    };
    let valid_domain = cfg.valid_domains.iter().any(|d| {
        let d = d.to_ascii_lowercase();
        match d.strip_prefix('*') {
            // `*.contoso.com`: exactly one more label, and only without a port.
            Some(suffix) => {
                port.is_none()
                    && suffix.starts_with('.')
                    && suffix.len() > 1
                    && host
                        .strip_suffix(suffix)
                        .is_some_and(|label| !label.is_empty() && !label.contains('.'))
            }
            None => d == authority,
        }
    });
    if !same_page && !valid_domain {
        return Err(format!(
            "the app resource {resource} is not on the domain of this page ({})",
            req.origin
        ));
    }
    req.resources = vec![resource];
    Ok(req)
}

/// `(host, port)` of an `api://<domain>[:port]/<app id>` resource.
fn app_resource_authority(resource: &str) -> Result<(String, Option<u16>), String> {
    let bad = || format!("{resource:?} is not an app resource of the form api://<domain>/<app id>");
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | ':');
    if !resource.chars().all(plain) {
        return Err(bad());
    }
    let u = url::Url::parse(resource).map_err(|_| bad())?;
    let host = u.host_str().ok_or_else(bad)?.to_ascii_lowercase();
    let app_id = u.path().trim_start_matches('/');
    if u.scheme() != "api" || !u.username().is_empty() || u.password().is_some() {
        return Err(bad());
    }
    if app_id.is_empty() || app_id.ends_with('/') {
        return Err(format!(
            "{resource} has no domain, so kelma cannot tell which page it belongs to"
        ));
    }
    Ok((host, u.port()))
}

/// The `claims` request parameter for a token: the JSON objects among teams-js `claims`
/// (conditional access challenges) merged into one, or `None`. Entries that are not JSON
/// objects are ignored.
pub fn merge_claims(claims: &[String]) -> Option<String> {
    fn merge(into: &mut serde_json::Map<String, Value>, from: serde_json::Map<String, Value>) {
        for (k, v) in from {
            match (into.get_mut(&k), v) {
                (Some(Value::Object(a)), Value::Object(b)) => merge(a, b),
                (_, v) => {
                    into.insert(k, v);
                }
            }
        }
    }
    let mut out = serde_json::Map::new();
    for c in claims {
        if let Ok(Value::Object(o)) = serde_json::from_str(c) {
            merge(&mut out, o);
        }
    }
    (!out.is_empty()).then(|| Value::Object(out).to_string())
}

/// The reason a failed app token request gives the app, from the sign-in error text. Teams
/// answers with these names, so apps can fall back to their own sign-in popup on consent and
/// multi-factor errors.
pub fn token_failure_reason(error: &str) -> String {
    let codes: Vec<u32> = error
        .match_indices("AADSTS")
        .filter_map(|(i, _)| {
            let digits: String = error[i + 6..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
        .collect();
    let any = |list: &[u32]| codes.iter().any(|c| list.contains(c));
    let reason = if any(&[50076, 50079]) {
        "resourceRequiresMfa"
    } else if any(&[50173, 70043, 700082]) {
        "tokenRevoked"
    } else if any(&[500011, 65002, 650057]) || error.contains("invalid_resource") {
        "resourceDisabled"
    } else if any(&[65001, 65004])
        || error.contains("consent_required")
        || error.contains("interaction_required")
    {
        "resourceRequiresConsent"
    } else {
        return format!("kelma could not get a token: {error}");
    };
    reason.to_string()
}

/// The response to an `authentication.getAuthToken` request.
pub fn token_response(to: &ReplyTo, result: Result<String, String>) -> Value {
    match result {
        Ok(token) => response(to, json!([true, token])),
        Err(reason) => response(to, json!([false, reason])),
    }
}

/// The host event that tells the app the theme changed.
pub fn theme_change(theme: &str) -> Value {
    json!({ "func": "themeChange", "args": [theme] })
}

/// JavaScript that delivers a response or host event to teams-js in the page.
pub fn deliver_js(msg: &Value) -> String {
    format!(
        "window.onNativeMessage&&window.onNativeMessage({})",
        json!({ "data": msg })
    )
}

/// Fills the `{placeholders}` of a tab `contentUrl` (`{locale}`, `{tid}`, `{entityId}`, ...).
/// Unknown placeholders are left as they are.
pub fn expand_url(template: &str, ctx: &HostContext) -> String {
    let values = [
        ("locale", ctx.locale.as_str()),
        ("tid", &ctx.tenant_id),
        ("entityId", &ctx.entity_id),
        ("subEntityId", ctx.sub_entity_id.as_deref().unwrap_or("")),
        ("userObjectId", &ctx.user_object_id),
        ("loginHint", &ctx.user_principal_name),
        ("userPrincipalName", &ctx.user_principal_name),
        ("theme", &ctx.theme),
        ("hostClientType", "desktop"),
        ("appSessionId", &ctx.app_session_id),
    ];
    let mut out = template.to_string();
    for (name, value) in values {
        let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
        out = out.replace(&format!("{{{name}}}"), &encoded);
    }
    out
}

/// A `window.kelma.invoke(payload)` call: `(call id, payload)`.
pub fn parse_bridge(raw: &str) -> Option<(u64, Value)> {
    let v: Value = serde_json::from_str(raw).ok()?;
    Some((v["id"].as_u64()?, v["payload"].clone()))
}

/// JavaScript that settles the promise of bridge call `id`.
pub fn bridge_reply_js(id: u64, result: &Result<Value, String>) -> String {
    let (ok, value) = match result {
        Ok(v) => (true, v.clone()),
        Err(e) => (false, json!(e)),
    };
    format!("window.kelma&&window.kelma.__settle({id},{ok},{value})")
}

/// The script injected into every new document.
pub fn injected_script(shim: bool, title: Option<&str>) -> String {
    let mut s = String::new();
    if shim {
        s.push_str("window.__kelmaShim=true;");
    }
    if let Some(t) = title {
        s.push_str(&format!("window.__kelmaTitle={};", json!(t)));
    }
    s.push_str(SHIM_JS);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ShimConfig {
        ShimConfig {
            context: HostContext {
                app_id: "app-1".into(),
                entity_id: "home".into(),
                user_object_id: "user-oid".into(),
                user_principal_name: "me@contoso.com".into(),
                user_display_name: "Me".into(),
                tenant_id: "tenant-1".into(),
                theme: "dark".into(),
                locale: "ar-sa".into(),
                app_session_id: "session-1".into(),
                ..HostContext::default()
            },
            allowed_origins: vec!["https://app.example.com".into()],
            ..ShimConfig::default()
        }
    }

    const ORIGIN: &str = "https://app.example.com";

    fn request(id: i64, func: &str, args: Value) -> String {
        json!({
            "id": id,
            "uuidAsString": format!("uuid-{id}"),
            "func": func,
            "args": args,
            "apiVersionTag": "v2_app.initialize",
            "timestamp": 1,
        })
        .to_string()
    }

    #[test]
    fn initialize_reports_a_desktop_content_frame_and_a_v4_runtime() {
        let h = handle(
            &cfg(),
            &request(0, "initialize", json!(["2.59.0", 4])),
            ORIGIN,
        );
        let reply = h.reply.expect("initialize is answered");
        assert_eq!(reply["id"], 0);
        assert_eq!(reply["uuidAsString"], "uuid-0");
        let args = reply["args"].as_array().unwrap();
        assert_eq!(args[0], "content");
        assert_eq!(args[1], "desktop");
        let runtime: Value = serde_json::from_str(args[2].as_str().unwrap()).unwrap();
        assert_eq!(runtime["apiVersion"], 4);
        assert!(runtime["supports"]["authentication"].is_object());
        assert_eq!(args[3], HOST_SDK_VERSION);
        assert_eq!(h.event, None);
        assert_eq!(h.token, None);
    }

    #[test]
    fn get_context_returns_the_legacy_context() {
        let h = handle(&cfg(), &request(1, "getContext", json!([])), ORIGIN);
        let reply = h.reply.unwrap();
        assert_eq!(reply["id"], 1);
        let c = &reply["args"][0];
        assert_eq!(c["frameContext"], "content");
        assert_eq!(c["hostClientType"], "desktop");
        assert_eq!(c["theme"], "dark");
        assert_eq!(c["locale"], "ar-sa");
        assert_eq!(c["appId"], "app-1");
        assert_eq!(c["entityId"], "home");
        assert_eq!(c["tid"], "tenant-1");
        assert_eq!(c["userObjectId"], "user-oid");
        assert_eq!(c["userPrincipalName"], "me@contoso.com");
        assert_eq!(c["loginHint"], "me@contoso.com");
        assert_eq!(c["userDisplayName"], "Me");
        assert_eq!(c["appSessionId"], "session-1");
        assert_eq!(c["hostName"], "Teams");
    }

    #[test]
    fn get_auth_token_is_routed_to_the_token_callback() {
        let args = json!([["api://app.example.com/abc"], ["c1"], true, "tenant-1"]);
        let h = handle(
            &cfg(),
            &request(7, "authentication.getAuthToken", args),
            ORIGIN,
        );
        assert_eq!(h.reply, None, "answered later, by the token callback");
        let (req, to) = h.token.expect("token request");
        assert_eq!(
            req,
            TokenRequest {
                resources: vec!["api://app.example.com/abc".into()],
                claims: vec!["c1".into()],
                silent: true,
                tenant_id: Some("tenant-1".into()),
                origin: ORIGIN.into(),
            }
        );
        assert_eq!(
            to,
            ReplyTo {
                id: 7,
                uuid: Some("uuid-7".into())
            }
        );
    }

    #[test]
    fn get_auth_token_without_arguments_is_a_plain_request() {
        let h = handle(
            &cfg(),
            &request(
                8,
                "authentication.getAuthToken",
                json!([null, null, null, null]),
            ),
            ORIGIN,
        );
        let (req, _) = h.token.unwrap();
        assert!(req.resources.is_empty() && req.claims.is_empty());
        assert!(!req.silent);
        assert_eq!(req.tenant_id, None);
    }

    #[test]
    fn token_response_carries_success_and_the_token_or_failure_and_the_reason() {
        let to = ReplyTo {
            id: 7,
            uuid: Some("uuid-7".into()),
        };
        let ok = token_response(&to, Ok("eyJ.token".into()));
        assert_eq!(
            ok,
            json!({"id": 7, "uuidAsString": "uuid-7", "args": [true, "eyJ.token"]})
        );
        let err = token_response(&to, Err("consent required".into()));
        assert_eq!(err["args"], json!([false, "consent required"]));
    }

    fn token_req(resources: &[&str], origin: &str) -> TokenRequest {
        TokenRequest {
            resources: resources.iter().map(|r| r.to_string()).collect(),
            claims: Vec::new(),
            silent: false,
            tenant_id: None,
            origin: origin.into(),
        }
    }

    fn checked(cfg: &ShimConfig, resources: &[&str], origin: &str) -> Result<String, String> {
        check_token_request(cfg, token_req(resources, origin)).map(|r| {
            assert_eq!(r.resources.len(), 1, "one validated resource");
            r.resources[0].clone()
        })
    }

    #[test]
    fn app_resources_on_the_page_origin_are_accepted() {
        let c = cfg();
        for r in [
            "api://app.example.com/0d5e1a2b-3c4d",
            "api://app.example.com/botid-0d5e1a2b-3c4d",
            "api://App.Example.com/0d5e1a2b-3c4d",
        ] {
            assert_eq!(checked(&c, &[r], ORIGIN), Ok(r.to_string()), "{r}");
        }
        assert_eq!(
            checked(
                &c,
                &["api://app.example.com:8443/abc"],
                "https://app.example.com:8443"
            ),
            Ok("api://app.example.com:8443/abc".into())
        );
        assert_eq!(
            checked(
                &c,
                &["api://app.example.com/abc"],
                "https://app.example.com:443"
            ),
            Ok("api://app.example.com/abc".into()),
            "the default port is the same origin"
        );
    }

    #[test]
    fn app_resources_of_another_origin_are_refused() {
        let c = cfg();
        let refused = |resources: &[&str], origin: &str| {
            let e = checked(&c, resources, origin).expect_err(&format!("{resources:?} {origin}"));
            assert!(!e.is_empty());
            e
        };
        // Another host, and lookalikes either way round.
        assert!(refused(&["api://other.example.com/abc"], ORIGIN).contains("app.example.com"));
        refused(&["api://example.com/abc"], ORIGIN);
        refused(&["api://app.example.com.evil.com/abc"], ORIGIN);
        refused(
            &["api://app.example.com/abc"],
            "https://app.example.com.evil.com",
        );
        refused(&["api://evil.com/app.example.com"], ORIGIN);
        refused(&["api://app.example.com@evil.com/abc"], ORIGIN);
        // Plain http, and a port the resource does not name (or the other way round).
        refused(&["api://app.example.com/abc"], "http://app.example.com");
        refused(
            &["api://app.example.com/abc"],
            "https://app.example.com:8443",
        );
        refused(&["api://app.example.com:8443/abc"], ORIGIN);
        // Forms without a domain, first-party resources and broken ones.
        refused(&["api://0d5e1a2b-3c4d"], ORIGIN);
        refused(&["api://botid-0d5e1a2b-3c4d"], ORIGIN);
        refused(&["api://app.example.com"], ORIGIN);
        refused(&["api://app.example.com/"], ORIGIN);
        refused(&["https://graph.microsoft.com"], ORIGIN);
        refused(&["https://app.example.com/abc"], ORIGIN);
        refused(&["api://app.example.com/abc?x=1"], ORIGIN);
        refused(&["api://app.example.com/a b"], ORIGIN);
        refused(&["not a url"], ORIGIN);
        // One resource per request.
        refused(&[], ORIGIN);
        refused(
            &["api://app.example.com/a", "api://app.example.com/b"],
            ORIGIN,
        );
    }

    #[test]
    fn the_manifest_resource_is_the_only_one_served_when_known() {
        let mut c = cfg();
        c.sso_resource = Some("api://app.example.com/abc".into());
        assert_eq!(
            checked(&c, &[], ORIGIN),
            Ok("api://app.example.com/abc".into())
        );
        assert_eq!(
            checked(&c, &["api://app.example.com/abc"], ORIGIN),
            Ok("api://app.example.com/abc".into())
        );
        assert!(checked(&c, &["api://app.example.com/other"], ORIGIN).is_err());
        // The manifest resource is still bound to the page origin.
        c.sso_resource = Some("api://elsewhere.example.net/abc".into());
        assert!(checked(&c, &[], ORIGIN).is_err());
    }

    #[test]
    fn valid_domains_from_the_manifest_extend_the_page_host() {
        let mut c = cfg();
        c.valid_domains = vec!["contoso.com".into(), "*.tabs.contoso.net".into()];
        assert!(checked(&c, &["api://contoso.com/abc"], ORIGIN).is_ok());
        assert!(checked(&c, &["api://a.tabs.contoso.net/abc"], ORIGIN).is_ok());
        assert!(checked(&c, &["api://www.contoso.com/abc"], ORIGIN).is_err());
        assert!(checked(&c, &["api://tabs.contoso.net/abc"], ORIGIN).is_err());
        assert!(checked(&c, &["api://a.tabs.contoso.net.evil.com/abc"], ORIGIN).is_err());
        assert!(checked(&c, &["api://evil-tabs.contoso.net/abc"], ORIGIN).is_err());
        // Still https only.
        assert!(checked(&c, &["api://contoso.com/abc"], "http://app.example.com").is_err());
        // A bare wildcard allows nothing.
        c.valid_domains = vec!["*".into()];
        assert!(checked(&c, &["api://evil.com/abc"], ORIGIN).is_err());
    }

    #[test]
    fn a_token_for_another_tenant_is_refused() {
        let c = cfg();
        let mut req = token_req(&["api://app.example.com/abc"], ORIGIN);
        req.tenant_id = Some("TENANT-1".into());
        assert!(check_token_request(&c, req.clone()).is_ok());
        req.tenant_id = Some("other-tenant".into());
        assert!(check_token_request(&c, req).is_err());
    }

    #[test]
    fn claims_challenges_are_merged_into_one_claims_parameter() {
        assert_eq!(merge_claims(&[]), None);
        assert_eq!(merge_claims(&["email".into(), "".into()]), None, "not json");
        let one = r#"{"access_token":{"acrs":{"essential":true,"value":"c1"}}}"#;
        let merged: Value = serde_json::from_str(&merge_claims(&[one.into()]).unwrap()).unwrap();
        assert_eq!(merged, serde_json::from_str::<Value>(one).unwrap());
        let two = r#"{"access_token":{"xms_cc":{"values":["cp1"]}}}"#;
        let merged: Value =
            serde_json::from_str(&merge_claims(&[one.into(), "junk".into(), two.into()]).unwrap())
                .unwrap();
        assert_eq!(
            merged,
            json!({"access_token": {
                "acrs": {"essential": true, "value": "c1"},
                "xms_cc": {"values": ["cp1"]}
            }})
        );
    }

    #[test]
    fn sign_in_errors_become_teams_js_reasons() {
        let cases = [
            (
                "auth: invalid_grant: AADSTS65001: The user or administrator has not consented",
                "resourceRequiresConsent",
            ),
            (
                "auth: consent_required: AADSTS65004: User declined to consent",
                "resourceRequiresConsent",
            ),
            (
                "auth: interaction_required: AADSTS50158: External security challenge not satisfied",
                "resourceRequiresConsent",
            ),
            (
                "auth: interaction_required: AADSTS50076: Due to a configuration change made by your administrator, you must use multi-factor authentication",
                "resourceRequiresMfa",
            ),
            (
                "auth: invalid_grant: AADSTS50079: Due to a configuration change, you must enroll in multi-factor authentication",
                "resourceRequiresMfa",
            ),
            (
                "auth: invalid_resource: AADSTS500011: The resource principal named api://x.com/abc was not found",
                "resourceDisabled",
            ),
            (
                "auth: invalid_client: AADSTS65002: Consent between first party application and first party resource must be configured via preauthorization",
                "resourceDisabled",
            ),
            (
                "auth: invalid_grant: AADSTS700082: The refresh token has expired due to inactivity",
                "tokenRevoked",
            ),
            (
                "auth: invalid_grant: AADSTS50173: The provided grant has expired due to it being revoked",
                "tokenRevoked",
            ),
        ];
        for (err, reason) in cases {
            assert_eq!(token_failure_reason(err), reason, "{err}");
        }
        let other = token_failure_reason("http: error sending request");
        assert!(other.contains("error sending request"), "{other}");
    }

    #[test]
    fn lifecycle_notifications_become_events_without_a_reply() {
        let c = cfg();
        let cases = [
            (
                "appInitialization.appLoaded",
                json!(["2.59.0"]),
                AppEvent::Loaded,
            ),
            (
                "appInitialization.success",
                json!(["2.59.0"]),
                AppEvent::Success,
            ),
            (
                "appInitialization.failure",
                json!(["AuthFailed", "no token", null]),
                AppEvent::Failure {
                    reason: "AuthFailed".into(),
                    message: "no token".into(),
                },
            ),
            (
                "appInitialization.expectedFailure",
                json!(["PermissionError", "nope"]),
                AppEvent::ExpectedFailure {
                    reason: "PermissionError".into(),
                    message: "nope".into(),
                },
            ),
        ];
        for (func, args, event) in cases {
            let h = handle(&c, &request(3, func, args), ORIGIN);
            assert_eq!(h.reply, None, "{func}");
            assert_eq!(h.event, Some(event), "{func}");
        }
    }

    #[test]
    fn register_handler_needs_no_reply() {
        let h = handle(
            &cfg(),
            &request(4, "registerHandler", json!(["themeChange"])),
            ORIGIN,
        );
        assert_eq!(h, Handled::default());
    }

    #[test]
    fn open_link_is_reported_and_acknowledged() {
        let url = "https://teams.microsoft.com/l/chat/0/0?users=a@b.c";
        let h = handle(&cfg(), &request(5, "executeDeepLink", json!([url])), ORIGIN);
        assert_eq!(h.event, Some(AppEvent::OpenLink(url.into())));
        assert_eq!(h.reply.unwrap()["args"], json!([true]));
    }

    #[test]
    fn unknown_calls_get_a_not_supported_sdk_error() {
        let h = handle(&cfg(), &request(6, "media.captureImage", json!([])), ORIGIN);
        let reply = h.reply.unwrap();
        assert_eq!(reply["args"][0]["errorCode"], 100);
        assert_eq!(
            h.event,
            Some(AppEvent::Unsupported("media.captureImage".into()))
        );
    }

    #[test]
    fn messages_from_other_origins_are_dropped() {
        for func in ["initialize", "getContext", "authentication.getAuthToken"] {
            let h = handle(
                &cfg(),
                &request(1, func, json!([])),
                "https://evil.example.net",
            );
            assert_eq!(h, Handled::default(), "{func}");
        }
    }

    #[test]
    fn malformed_messages_are_ignored() {
        for raw in ["", "nope", "[]", "{\"func\": 3}", "{\"id\": 1}"] {
            assert_eq!(handle(&cfg(), raw, ORIGIN), Handled::default(), "{raw}");
        }
    }

    #[test]
    fn responses_without_uuid_are_matched_by_id() {
        let raw = json!({"id": 2, "func": "getContext", "args": []}).to_string();
        let reply = handle(&cfg(), &raw, ORIGIN).reply.unwrap();
        assert_eq!(reply["id"], 2);
        assert!(reply.get("uuidAsString").is_none_or(Value::is_null));
    }

    #[test]
    fn theme_change_is_a_host_event() {
        assert_eq!(
            theme_change("dark"),
            json!({"func": "themeChange", "args": ["dark"]})
        );
    }

    #[test]
    fn deliver_js_calls_on_native_message_with_the_message_as_data() {
        let msg = json!({"id": 1, "args": ["</script>"]});
        let js = deliver_js(&msg);
        assert!(js.starts_with("window.onNativeMessage&&window.onNativeMessage("));
        assert!(js.contains(&json!({ "data": msg }).to_string()), "{js}");
    }

    #[test]
    fn for_app_allows_the_tab_origin_and_exact_valid_domains_only() {
        let ctx = HostContext::default();
        let cfg = ShimConfig::for_app(
            ctx,
            "https://tabs.contoso.com/home?x=1",
            Some("api://tabs.contoso.com/0d5e".into()),
            vec![
                "tabs.contoso.com".into(),
                "api.contoso.com".into(),
                "*.fabrikam.com".into(),
                "localhost:3000".into(),
                "evil.com/path".into(),
                "".into(),
            ],
        );
        assert_eq!(
            cfg.allowed_origins,
            [
                "https://tabs.contoso.com",
                "https://api.contoso.com",
                "https://localhost:3000"
            ]
        );
        assert_eq!(
            cfg.sso_resource.as_deref(),
            Some("api://tabs.contoso.com/0d5e")
        );
        // The wildcard is kept for the token check.
        assert!(cfg.valid_domains.iter().any(|d| d == "*.fabrikam.com"));
        assert_eq!(origin_of("data:text/html,x"), None);
        assert_eq!(origin_of("not a url"), None);
    }

    #[test]
    fn expand_url_fills_the_known_placeholders() {
        let ctx = cfg().context;
        let url = expand_url(
            "https://x.test/?l={locale}&t={tid}&e={entityId}&u={userObjectId}&h={loginHint}&th={theme}&s={subEntityId}&z={unknown}",
            &ctx,
        );
        assert_eq!(
            url,
            "https://x.test/?l=ar-sa&t=tenant-1&e=home&u=user-oid&h=me%40contoso.com&th=dark&s=&z={unknown}"
        );
    }

    #[test]
    fn bridge_calls_are_parsed_and_settled() {
        assert_eq!(
            parse_bridge(r#"{"id": 4, "payload": {"ping": 1}}"#),
            Some((4, json!({"ping": 1})))
        );
        assert_eq!(parse_bridge("garbage"), None);
        assert_eq!(parse_bridge(r#"{"payload": 1}"#), None);
        let ok = bridge_reply_js(4, &Ok(json!({"pong": 1})));
        assert_eq!(
            ok,
            r#"window.kelma&&window.kelma.__settle(4,true,{"pong":1})"#
        );
        let err = bridge_reply_js(5, &Err("boom".into()));
        assert_eq!(
            err,
            r#"window.kelma&&window.kelma.__settle(5,false,"boom")"#
        );
    }

    #[test]
    fn injected_script_enables_the_shim_and_title_only_when_asked() {
        let with = injected_script(true, Some("Polly \"app\""));
        assert!(with.starts_with("window.__kelmaShim=true;"));
        assert!(with.contains(r#"window.__kelmaTitle="Polly \"app\"";"#));
        assert!(with.contains(SHIM_JS));
        let without = injected_script(false, None);
        assert!(!without.contains("__kelmaShim=true"));
        assert!(!without.contains("__kelmaTitle="));
        assert!(without.contains(SHIM_JS));
    }
}
