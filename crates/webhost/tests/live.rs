//! Live checks against a real Chromium-family browser. They open windows on the desktop, so
//! they are ignored by default (CI has no browser):
//!
//! ```text
//! cargo test -p webhost --test live -- --ignored --test-threads=1
//! ```
//!
//! - `KELMA_BROWSER`: browser to use (default: first Chromium-family browser on PATH)
//! - `KELMA_WEBHOST_TEST_PROFILE`: profile directory (default: a temporary directory)
//! - `KELMA_TEAMSJS`: path to `@microsoft/teams-js`'s `dist/umd/MicrosoftTeams.min.js`; without
//!   it the page uses a small client that speaks the same wire protocol.
//!
//! The pages are served from 127.0.0.1 and need no sign-in.

use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use webhost::{AppEvent, BridgeCall, HostContext, ShimConfig, TokenRequest, WebOptions};

/// The parts of teams-js 2.x this test uses, speaking its frameless wire protocol.
const MINI_TEAMS_JS: &str = r#"
(function () {
  var next = 0, waiting = {}, handlers = {};
  function send(func, args, cb) {
    var id = next++, uuid = "u-" + id + "-" + Math.random();
    if (cb) waiting[uuid] = cb;
    window.nativeInterface.framelessPostMessage(JSON.stringify({
      id: id, uuidAsString: uuid, func: func, args: args || [],
      apiVersionTag: "v2_test", timestamp: Date.now()
    }));
  }
  function call(func, args) { return new Promise(function (r) { send(func, args, r); }); }
  window.onNativeMessage = function (evt) {
    var m = evt.data;
    if (typeof m.id === "number") {
      var cb = waiting[m.uuidAsString];
      if (cb) { delete waiting[m.uuidAsString]; cb(m.args); }
    } else if (typeof m.func === "string" && handlers[m.func]) {
      handlers[m.func].apply(null, m.args);
    }
  };
  window.microsoftTeams = {
    app: {
      initialize: function () {
        if (!window.nativeInterface) return Promise.reject(new Error("No Parent window found."));
        return call("initialize", ["2.59.0", 4]).then(function () {});
      },
      getContext: function () {
        return call("getContext").then(function (a) {
          var c = a[0];
          return { app: { locale: c.locale, theme: c.theme, host: { name: c.hostName, clientType: c.hostClientType } },
                   page: { id: c.entityId, frameContext: c.frameContext },
                   user: { id: c.userObjectId, tenant: { id: c.tid } } };
        });
      },
      notifySuccess: function () { send("appInitialization.success", ["2.59.0"]); },
      registerOnThemeChangeHandler: function (h) {
        handlers.themeChange = h; send("registerHandler", ["themeChange"]);
      }
    },
    authentication: {
      getAuthToken: function (r) {
        return call("authentication.getAuthToken", [r && r.resources, r && r.claims, r && r.silent, r && r.tenantId])
          .then(function (a) { if (a[0]) return a[1]; throw new Error(a[1]); });
      }
    }
  };
})();
"#;

const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>webhost live</title>
<script src="/teams.js"></script>
<p>kelma web host live check</p>
<script>
window.lastTheme = null;
window.results = (async () => {
  await microsoftTeams.app.initialize();
  const ctx = await microsoftTeams.app.getContext();
  microsoftTeams.app.registerOnThemeChangeHandler(t => { window.lastTheme = t; });
  // The page is plain http, so the host must refuse to mint an app token for it.
  const token = await microsoftTeams.authentication.getAuthToken({ resources: ["api://127.0.0.1/test"] })
    .then(t => "minted:" + t, e => "refused: " + String(e && e.message || e));
  microsoftTeams.app.notifySuccess();
  const echo = await window.kelma.invoke({ ping: 1 });
  return {
    host: ctx.app.host.name, clientType: ctx.app.host.clientType,
    frameContext: ctx.page.frameContext, locale: ctx.app.locale, theme: ctx.app.theme,
    entityId: ctx.page.id, tid: ctx.user.tenant.id, token, echo
  };
})().catch(e => ({ error: String(e && e.message || e) }));
</script>"#;

/// Serves `/` (the page) and `/teams.js` on 127.0.0.1; returns the base URL.
async fn serve(teams_js: String) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let teams_js = Arc::new(teams_js);
    tokio::spawn(async move {
        while let Ok((mut tcp, _)) = listener.accept().await {
            let teams_js = teams_js.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = tcp.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/");
                let (status, kind, body) = match path {
                    "/" => ("200 OK", "text/html; charset=utf-8", PAGE.to_string()),
                    "/teams.js" => ("200 OK", "text/javascript", teams_js.to_string()),
                    _ => ("404 Not Found", "text/plain", String::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tcp.write_all(head.as_bytes()).await;
                let _ = tcp.write_all(body.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

fn profile() -> (std::path::PathBuf, Option<tempfile::TempDir>) {
    match std::env::var_os("KELMA_WEBHOST_TEST_PROFILE") {
        Some(p) => (p.into(), None),
        None => {
            let d = tempfile::tempdir().unwrap();
            (d.path().join("webhost-profile"), Some(d))
        }
    }
}

fn options(
    profile: std::path::PathBuf,
    tokens: Arc<std::sync::Mutex<Vec<TokenRequest>>>,
) -> WebOptions {
    let mut opts = WebOptions::new(profile);
    opts.title = Some("kelma web host live check".into());
    opts.size = Some((900, 650));
    opts.shim = Some(ShimConfig {
        context: HostContext {
            app_id: "live-app".into(),
            entity_id: "home".into(),
            tenant_id: "tenant-live".into(),
            locale: "ar-sa".into(),
            theme: "dark".into(),
            ..HostContext::default()
        },
        allowed_origins: Vec::new(),
        ..ShimConfig::default()
    });
    opts.token = Some(Arc::new(move |req: TokenRequest| {
        let tokens = tokens.clone();
        Box::pin(async move {
            let token = format!("fake-token-for:{}", req.resources.join(","));
            tokens.lock().unwrap().push(req);
            Ok(token)
        })
    }));
    opts
}

async fn poll(win: &webhost::WebWindow, js: &str, want: &Value) -> Value {
    let mut last = Value::Null;
    for _ in 0..100 {
        last = win.eval(js).await.unwrap();
        if &last == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    last
}

#[tokio::test]
#[ignore = "opens a browser window"]
async fn shim_answers_teams_js_and_the_bridge_round_trips() {
    let teams_js = match std::env::var("KELMA_TEAMSJS") {
        Ok(p) => std::fs::read_to_string(p).expect("read KELMA_TEAMSJS"),
        Err(_) => MINI_TEAMS_JS.to_string(),
    };
    let base = serve(teams_js).await;
    let (profile, _tmp) = profile();
    let tokens = Arc::new(std::sync::Mutex::new(Vec::new()));
    let win = webhost::open(&format!("{base}/"), options(profile, tokens.clone()))
        .await
        .expect("open the host window");
    let mut events = win.events();
    win.on_message(Arc::new(|call: BridgeCall| {
        Box::pin(async move { Ok(json!({ "pong": call.payload["ping"], "origin": call.origin })) })
    }));

    let results = tokio::time::timeout(
        Duration::from_secs(20),
        win.eval("window.results || new Promise(r => setTimeout(() => r(window.results), 1000))"),
    )
    .await
    .expect("page answered in time")
    .unwrap();
    println!("page results: {results}");
    assert_eq!(results["error"], Value::Null, "{results}");
    assert_eq!(results["host"], "Teams");
    assert_eq!(results["clientType"], "desktop");
    assert_eq!(results["frameContext"], "content");
    assert_eq!(results["locale"], "ar-sa");
    assert_eq!(results["theme"], "dark");
    assert_eq!(results["entityId"], "home");
    assert_eq!(results["tid"], "tenant-live");
    let token = results["token"].as_str().unwrap_or_default();
    assert!(
        token.starts_with("refused: ") && token.contains("https"),
        "{token}"
    );
    assert_eq!(results["echo"], json!({ "pong": 1, "origin": base }));

    assert!(
        tokens.lock().unwrap().is_empty(),
        "a refused request never reaches the token callback"
    );

    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap() {
                AppEvent::Success => return AppEvent::Success,
                _ => continue,
            }
        }
    })
    .await
    .expect("notifySuccess reached Rust");
    assert_eq!(event, AppEvent::Success);

    win.set_theme("contrast").await.unwrap();
    assert_eq!(
        poll(&win, "window.lastTheme", &json!("contrast")).await,
        json!("contrast")
    );

    let title = poll(&win, "document.title", &json!("kelma web host live check")).await;
    assert_eq!(title, "kelma web host live check");

    tokio::time::timeout(Duration::from_secs(10), win.close())
        .await
        .expect("window closed in time");
    assert!(win.is_closed());
    assert!(matches!(win.eval("1").await, Err(webhost::Error::Closed)));
}

#[tokio::test]
#[ignore = "opens a browser window"]
async fn closing_the_window_resolves_closed_and_stops_the_browser() {
    let base = serve(MINI_TEAMS_JS.to_string()).await;
    let (profile, _tmp) = profile();
    let win = webhost::open(
        &format!("{base}/"),
        options(profile.clone(), Default::default()),
    )
    .await
    .expect("open the host window");
    assert!(!win.is_closed());

    assert_eq!(
        poll(&win, "document.readyState", &json!("complete")).await,
        "complete"
    );
    // Close the page target from a second DevTools client: what the window's close button does.
    let port = std::fs::read_to_string(profile.join("DevToolsActivePort")).unwrap();
    let (port, path) = webhost::browser::parse_devtools_active_port(&port).unwrap();
    let (other, _events) = webhost::cdp::Cdp::connect(&format!("ws://127.0.0.1:{port}{path}"))
        .await
        .unwrap();
    let targets = other
        .call("Target.getTargets", json!({}), None)
        .await
        .unwrap();
    let page = targets["targetInfos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["type"] == "page" && t["url"].as_str().unwrap().starts_with(&base))
        .expect("the app page target")["targetId"]
        .clone();
    other
        .call("Target.closeTarget", json!({ "targetId": page }), None)
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(10), win.closed())
        .await
        .expect("closed() resolved after the window closed");
    assert!(win.is_closed());
    assert!(
        !webhost::browser::profile_in_use(&profile),
        "the browser exited and released the profile"
    );
}
