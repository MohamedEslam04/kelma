//! A host window: one browser process with its own profile, one `--app` window, one CDP
//! session to its page.

use crate::cdp::{Cdp, Event};
use crate::shim::{self, AppEvent, ShimConfig, TokenRequest};
use crate::{Error, Result, browser};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Answers `authentication.getAuthToken`: `Ok(access token)` or `Err(reason shown to the app)`.
/// IronTeams's UI plugs `teams_proto`'s auth broker in here. It is only called for requests that
/// passed [`shim::check_token_request`], so `resources` holds exactly the one resource to mint.
pub type TokenCallback =
    Arc<dyn Fn(TokenRequest) -> BoxFuture<std::result::Result<String, String>> + Send + Sync>;

/// Answers `window.ironteams.invoke(payload)` calls from the page.
pub type BridgeHandler =
    Arc<dyn Fn(BridgeCall) -> BoxFuture<std::result::Result<Value, String>> + Send + Sync>;

/// One `window.ironteams.invoke(payload)` call.
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeCall {
    pub payload: Value,
    /// Origin of the calling page, from the CDP execution context (the page cannot fake it).
    pub origin: String,
}

/// How to open a window.
#[derive(Clone)]
pub struct WebOptions {
    /// Window title, set when each page finishes parsing (the page may change it later).
    pub title: Option<String>,
    /// Initial window size in pixels.
    pub size: Option<(u32, u32)>,
    /// The browser profile. Sign-ins and cookies persist here, separate from the user's own
    /// browser profile. See [`default_profile_dir`].
    pub profile_dir: PathBuf,
    /// Browser executable; `None` searches `$IRONTEAMS_BROWSER`, then [`browser::CANDIDATES`].
    pub browser: Option<PathBuf>,
    /// TeamsJS host shim; `None` opens a plain page (the `window.ironteams` bridge still works).
    pub shim: Option<ShimConfig>,
    /// Token callback for the shim. Without one, `getAuthToken` fails.
    pub token: Option<TokenCallback>,
    /// How long to wait for the browser and its DevTools endpoint.
    pub startup_timeout: Duration,
}

impl WebOptions {
    pub fn new(profile_dir: impl Into<PathBuf>) -> Self {
        WebOptions {
            title: None,
            size: None,
            profile_dir: profile_dir.into(),
            browser: None,
            shim: None,
            token: None,
            startup_timeout: Duration::from_secs(20),
        }
    }
}

/// The host profile under IronTeams's data directory.
pub fn default_profile_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("webhost-profile")
}

struct Inner {
    cdp: Cdp,
    session: String,
    target: String,
    shim: Mutex<Option<ShimConfig>>,
    token: Option<TokenCallback>,
    bridge: Mutex<Option<BridgeHandler>>,
    events: broadcast::Sender<AppEvent>,
    closed: watch::Sender<bool>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    /// Process group of the browser (and its launcher script).
    pgid: u32,
    exited: AtomicBool,
    /// Execution context id -> security origin, from `Runtime.executionContextCreated`.
    contexts: Mutex<HashMap<i64, String>>,
}

/// An open host window. Dropping it closes the window and stops the browser.
pub struct WebWindow {
    inner: Arc<Inner>,
}

/// Opens `url` in a new host window.
pub async fn open(url: &str, opts: WebOptions) -> Result<WebWindow> {
    let exe = browser::find_browser(opts.browser.as_deref()).ok_or(Error::NoBrowser)?;
    tokio::fs::create_dir_all(&opts.profile_dir).await?;
    // A window closed just before may still be shutting down: give it a moment.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while browser::profile_in_use(&opts.profile_dir) {
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::ProfileInUse(opts.profile_dir.clone()));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let port_file = opts.profile_dir.join("DevToolsActivePort");
    match tokio::fs::remove_file(&port_file).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    // Its own process group: launchers such as /usr/bin/brave-browser are scripts that run the
    // browser as a child, so stopping only the direct child would leave the browser running.
    let mut command = tokio::process::Command::new(&exe);
    command
        .args(browser::launch_args(&opts.profile_dir, opts.size))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let pgid = child.id().ok_or(Error::Closed)?;
    let connected = tokio::time::timeout(
        opts.startup_timeout,
        connect(&mut child, &port_file, opts.startup_timeout),
    )
    .await
    .unwrap_or_else(|_| Err(Error::Startup("timed out connecting to DevTools".into())));
    let (cdp, events, target, session) = match connected {
        Ok(c) => c,
        Err(e) => {
            browser::signal_group(pgid, browser::Stop::Kill);
            return Err(e);
        }
    };

    let mut shim_cfg = opts.shim.clone();
    if let Some(cfg) = shim_cfg.as_mut().filter(|c| c.allowed_origins.is_empty()) {
        cfg.allowed_origins.extend(default_origin(url));
    }
    let (kill_tx, kill_rx) = oneshot::channel();
    let inner = Arc::new(Inner {
        cdp,
        session,
        target,
        shim: Mutex::new(shim_cfg),
        token: opts.token.clone(),
        bridge: Mutex::new(None),
        events: broadcast::channel(32).0,
        closed: watch::channel(false).0,
        kill: Mutex::new(Some(kill_tx)),
        pgid,
        exited: AtomicBool::new(false),
        contexts: Mutex::new(HashMap::new()),
    });
    tokio::spawn(watch_child(child, kill_rx, inner.clone()));
    tokio::spawn(run_events(inner.clone(), events));
    // From here on, dropping `win` (also on an error below) stops the browser.
    let win = WebWindow { inner };
    let shim_on = opts.shim.is_some();
    let granted = shim_cfg_granted(&win.inner);
    tokio::time::timeout(
        opts.startup_timeout,
        prepare(&win.inner, url, shim_on, opts.title.as_deref(), granted),
    )
    .await
    .unwrap_or_else(|_| Err(Error::Startup("timed out preparing the page".into())))?;
    Ok(win)
}

/// Registers the bindings and the injected script, then loads `url`.
async fn prepare(
    inner: &Inner,
    url: &str,
    shim_on: bool,
    title: Option<&str>,
    granted: Option<Vec<String>>,
) -> Result<()> {
    let s = Some(inner.session.as_str());
    inner.cdp.call("Page.enable", json!({}), s).await?;
    if let Some(granted) = &granted {
        grant_browser_permissions(inner, granted).await;
    }
    if shim_on {
        inner
            .cdp
            .call(
                "Runtime.addBinding",
                json!({ "name": shim::TEAMS_BINDING }),
                s,
            )
            .await?;
    }
    inner
        .cdp
        .call(
            "Runtime.addBinding",
            json!({ "name": shim::BRIDGE_BINDING }),
            s,
        )
        .await?;
    inner
        .cdp
        .call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": shim::injected_script(shim_on, title, granted.as_deref()) }),
            s,
        )
        .await?;
    inner.cdp.call("Runtime.enable", json!({}), s).await?;
    let nav = inner
        .cdp
        .call("Page.navigate", json!({ "url": url }), s)
        .await?;
    match nav["errorText"].as_str() {
        Some(e) => Err(Error::Startup(format!("could not load the page: {e}"))),
        None => Ok(()),
    }
}

/// The app's granted permissions when the window has a shim, else `None` (a plain page).
fn shim_cfg_granted(inner: &Inner) -> Option<Vec<String>> {
    let shim = inner.shim.lock().unwrap();
    shim.as_ref().map(|c| c.granted.clone())
}

/// Sets the browser's own permissions for the app's origins to exactly the granted set:
/// `Browser.grantPermissions` grants the listed ones and rejects every other, so the browser
/// never prompts for what the user did not allow (the injected script refuses the same calls
/// first). Failing to set them is not fatal: the script still refuses.
async fn grant_browser_permissions(inner: &Inner, granted: &[String]) {
    let permissions = shim::browser_permissions(granted);
    let origins = {
        let shim = inner.shim.lock().unwrap();
        shim.as_ref()
            .map(|c| c.allowed_origins.clone())
            .unwrap_or_default()
    };
    let _ = inner
        .cdp
        .call("Browser.resetPermissions", json!({}), None)
        .await;
    for origin in origins {
        let r = inner
            .cdp
            .call(
                "Browser.grantPermissions",
                json!({ "origin": origin, "permissions": permissions }),
                None,
            )
            .await;
        if let Err(e) = r {
            tracing::warn!(%origin, error = %e, "could not set the browser permissions");
        }
    }
}

/// Waits for the DevTools endpoint, connects and attaches to the window's page.
async fn connect(
    child: &mut tokio::process::Child,
    port_file: &Path,
    timeout: Duration,
) -> Result<(Cdp, mpsc::UnboundedReceiver<Event>, String, String)> {
    let ws_url = wait_for_devtools(child, port_file, timeout).await?;
    let (cdp, events) = Cdp::connect(&ws_url).await?;
    cdp.call(
        "Target.setDiscoverTargets",
        json!({ "discover": true }),
        None,
    )
    .await?;
    let target = wait_for_page(&cdp, timeout).await?;
    let attached = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target, "flatten": true }),
            None,
        )
        .await?;
    let session = attached["sessionId"]
        .as_str()
        .ok_or_else(|| Error::Startup("attachToTarget returned no session".into()))?
        .to_string();
    Ok((cdp, events, target, session))
}

async fn wait_for_devtools(
    child: &mut tokio::process::Child,
    port_file: &Path,
    timeout: Duration,
) -> Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(text) = tokio::fs::read_to_string(port_file).await
            && let Some((port, path)) = browser::parse_devtools_active_port(&text)
        {
            return Ok(format!("ws://127.0.0.1:{port}{path}"));
        }
        if let Some(status) = child.try_wait()? {
            return Err(Error::Startup(format!(
                "the browser exited ({status}) before opening DevTools"
            )));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Startup("timed out waiting for DevTools".into()));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_page(cdp: &Cdp, timeout: Duration) -> Result<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let targets = cdp.call("Target.getTargets", json!({}), None).await?;
        if let Some(id) = pick_page_target(&targets) {
            return Ok(id);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Startup("the browser opened no page".into()));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The [`browser::START_URL`] page the `--app` window starts on, from a `Target.getTargets` result.
fn pick_page_target(targets: &Value) -> Option<String> {
    targets["targetInfos"]
        .as_array()?
        .iter()
        .find(|t| t["type"] == "page" && t["url"] == browser::START_URL)
        .and_then(|t| t["targetId"].as_str())
        .map(str::to_string)
}

/// The origin shim messages are accepted from when none is configured.
fn default_origin(url: &str) -> Option<String> {
    let origin = url::Url::parse(url).ok()?.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

async fn watch_child(
    mut child: tokio::process::Child,
    kill: oneshot::Receiver<()>,
    inner: Arc<Inner>,
) {
    tokio::select! {
        _ = child.wait() => {}
        _ = kill => {}
    }
    // The launcher may exit before the browser it started, so wait for the whole group. `stop`
    // sent SIGTERM, which lets the browser save its profile (cookies) before it exits.
    let group_gone = async {
        let _ = child.wait().await;
        while browser::group_alive(inner.pgid) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    if tokio::time::timeout(Duration::from_secs(5), group_gone)
        .await
        .is_err()
    {
        browser::signal_group(inner.pgid, browser::Stop::Kill);
        let _ = child.wait().await;
    }
    inner.exited.store(true, Ordering::SeqCst);
    inner.closed.send_replace(true);
}

async fn run_events(inner: Arc<Inner>, mut events: mpsc::UnboundedReceiver<Event>) {
    while let Some(ev) = events.recv().await {
        let ours = ev.session_id.as_deref() == Some(inner.session.as_str());
        match ev.method.as_str() {
            "Runtime.executionContextCreated" if ours => {
                let c = &ev.params["context"];
                if let (Some(id), Some(origin)) = (c["id"].as_i64(), c["origin"].as_str()) {
                    inner
                        .contexts
                        .lock()
                        .unwrap()
                        .insert(id, origin.to_string());
                }
            }
            "Runtime.executionContextDestroyed" if ours => {
                if let Some(id) = ev.params["executionContextId"].as_i64() {
                    inner.contexts.lock().unwrap().remove(&id);
                }
            }
            "Runtime.executionContextsCleared" if ours => inner.contexts.lock().unwrap().clear(),
            "Runtime.bindingCalled" if ours => {
                let origin = ev.params["executionContextId"]
                    .as_i64()
                    .and_then(|id| inner.contexts.lock().unwrap().get(&id).cloned())
                    .unwrap_or_default();
                let name = ev.params["name"].as_str().unwrap_or("").to_string();
                let payload = ev.params["payload"].as_str().unwrap_or("").to_string();
                tokio::spawn(on_binding(inner.clone(), name, payload, origin));
            }
            "Inspector.detached" if ours => break,
            "Target.targetDestroyed" if ev.params["targetId"] == inner.target.as_str() => break,
            "Target.detachedFromTarget" if ev.params["sessionId"] == inner.session.as_str() => {
                break;
            }
            _ => {}
        }
    }
    // The page is gone (window closed, crash or lost connection): stop the browser too.
    stop(&inner);
}

/// Stops the browser: SIGTERM to its process group now (this also works while the runtime
/// shuts down), and SIGKILL from `watch_child` if it is still running after 5 s.
fn stop(inner: &Inner) {
    if let Some(kill) = inner.kill.lock().unwrap().take() {
        if !inner.exited.load(Ordering::SeqCst) {
            browser::signal_group(inner.pgid, browser::Stop::Term);
        }
        let _ = kill.send(());
    }
}

async fn on_binding(inner: Arc<Inner>, name: String, payload: String, origin: String) {
    let js = if name == shim::TEAMS_BINDING {
        let Some(cfg) = inner.shim.lock().unwrap().clone() else {
            return;
        };
        let handled = shim::handle(&cfg, &payload, &origin);
        if let Some(e) = handled.event {
            let _ = inner.events.send(e);
        }
        if let Some(reply) = handled.reply {
            let _ = evaluate(&inner, &shim::deliver_js(&reply)).await;
        }
        let Some((request, to)) = handled.token else {
            return;
        };
        // The app's own resource on the app's own origin, or nothing is minted.
        let result = match (&inner.token, shim::check_token_request(&cfg, request)) {
            (_, Err(reason)) => {
                tracing::info!(%origin, %reason, "app token request refused");
                Err(reason)
            }
            (Some(cb), Ok(request)) => cb(request).await,
            (None, Ok(_)) => Err("IronTeams has no token provider for this window".into()),
        };
        shim::deliver_js(&shim::token_response(&to, result))
    } else if name == shim::BRIDGE_BINDING {
        let Some((id, payload)) = shim::parse_bridge(&payload) else {
            return;
        };
        let handler = inner.bridge.lock().unwrap().clone();
        let result = match handler {
            Some(h) => h(BridgeCall { payload, origin }).await,
            None => Err("IronTeams has no handler for window.ironteams.invoke".into()),
        };
        shim::bridge_reply_js(id, &result)
    } else {
        return;
    };
    if let Err(e) = evaluate(&inner, &js).await {
        tracing::debug!(error = %e, "web host reply not delivered");
    }
}

async fn evaluate(inner: &Inner, js: &str) -> Result<Value> {
    let r = inner
        .cdp
        .call(
            "Runtime.evaluate",
            json!({ "expression": js, "returnByValue": true, "awaitPromise": true }),
            Some(&inner.session),
        )
        .await?;
    if let Some(ex) = r.get("exceptionDetails") {
        let msg = ex["exception"]["description"]
            .as_str()
            .or(ex["text"].as_str())
            .unwrap_or("exception");
        return Err(Error::Script(msg.to_string()));
    }
    Ok(r["result"]["value"].clone())
}

impl WebWindow {
    /// Evaluates `js` in the page (promises are awaited) and returns its JSON value.
    pub async fn eval(&self, js: &str) -> Result<Value> {
        if self.is_closed() {
            return Err(Error::Closed);
        }
        evaluate(&self.inner, js).await
    }

    /// Loads `url` in the window and brings the window to the front (a window kept open for
    /// the next page, like the recording player).
    pub async fn navigate(&self, url: &str) -> Result<()> {
        if self.is_closed() {
            return Err(Error::Closed);
        }
        let s = Some(self.inner.session.as_str());
        let nav = self
            .inner
            .cdp
            .call("Page.navigate", json!({ "url": url }), s)
            .await?;
        if let Some(e) = nav["errorText"].as_str() {
            return Err(Error::Startup(format!("could not load the page: {e}")));
        }
        let _ = self.inner.cdp.call("Page.bringToFront", json!({}), s).await;
        Ok(())
    }

    /// Sets the handler for `window.ironteams.invoke(payload)` calls, replacing any earlier one.
    pub fn on_message(&self, handler: BridgeHandler) {
        *self.inner.bridge.lock().unwrap() = Some(handler);
    }

    /// App lifecycle events from the TeamsJS shim (loaded, success, failure, links).
    pub fn events(&self) -> broadcast::Receiver<AppEvent> {
        self.inner.events.subscribe()
    }

    /// Tells the app the theme changed (`default`, `dark` or `contrast`); later `getContext`
    /// calls report it too.
    pub async fn set_theme(&self, theme: &str) -> Result<()> {
        if let Some(cfg) = self.inner.shim.lock().unwrap().as_mut() {
            cfg.context.theme = theme.to_string();
        }
        self.eval(&shim::deliver_js(&shim::theme_change(theme)))
            .await
            .map(|_| ())
    }

    /// Whether the window has closed.
    pub fn is_closed(&self) -> bool {
        *self.inner.closed.borrow()
    }

    /// Resolves once the window is closed (by the user, [`WebWindow::close`] or a crash).
    pub async fn closed(&self) {
        let mut rx = self.inner.closed.subscribe();
        let _ = rx.wait_for(|c| *c).await;
    }

    /// Closes the window and stops its browser.
    pub async fn close(&self) {
        let browser_close = self.inner.cdp.call("Browser.close", json!({}), None);
        let _ = tokio::time::timeout(Duration::from_secs(2), browser_close).await;
        stop(&self.inner);
        self.closed().await;
    }
}

impl Drop for WebWindow {
    fn drop(&mut self) {
        stop(&self.inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_start_page_and_skips_browser_ui_pages() {
        // Shaped like Vivaldi's targets: its own UI is an app plus a start page.
        let targets = json!({"targetInfos": [
            {"targetId": "W1", "type": "service_worker", "url": "chrome-extension://x/bg.js"},
            {"targetId": "A1", "type": "app", "url": "chrome-extension://x/window.html"},
            {"targetId": "P0", "type": "page", "url": "chrome://vivaldi-webui/startpage"},
            {"targetId": "P1", "type": "page", "url": browser::START_URL},
            {"targetId": "P2", "type": "page", "url": browser::START_URL},
        ]});
        assert_eq!(pick_page_target(&targets), Some("P1".into()));
        let only_ui = json!({"targetInfos": [
            {"targetId": "P0", "type": "page", "url": "chrome://vivaldi-webui/startpage"},
        ]});
        assert_eq!(pick_page_target(&only_ui), None);
        assert_eq!(pick_page_target(&json!({"targetInfos": []})), None);
        assert_eq!(pick_page_target(&json!({})), None);
    }

    #[test]
    fn default_origin_is_the_origin_of_the_opened_url() {
        assert_eq!(
            default_origin("https://app.example.com:8443/tab?x=1#y"),
            Some("https://app.example.com:8443".into())
        );
        assert_eq!(
            default_origin("http://127.0.0.1:4000/"),
            Some("http://127.0.0.1:4000".into())
        );
        assert_eq!(default_origin("data:text/html,hi"), None, "opaque origin");
        assert_eq!(default_origin("not a url"), None);
    }

    #[test]
    fn default_profile_dir_is_under_the_data_dir() {
        assert_eq!(
            default_profile_dir(Path::new("/home/u/.local/share/ironteams")),
            PathBuf::from("/home/u/.local/share/ironteams/webhost-profile")
        );
    }
}
