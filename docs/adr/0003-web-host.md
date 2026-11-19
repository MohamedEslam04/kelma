# ADR 0003: Web host

- Status: accepted (2026-11-19)
- Date: 2026-11-19

## Context

Some parts of Teams have no API kelma can draw natively: the "W" apps of
`plans/store-and-apps.md` (Approvals, Shifts, Whiteboard, Loop, Forms, Viva…), Office web
editing, meeting options and the Copilot fallback. They need a web view that:

- runs the Teams and Microsoft 365 web apps (Chromium is what Microsoft supports),
- keeps a sign-in between runs, without touching the user's own browser profile,
- answers TeamsJS (`@microsoft/teams-js`) calls from kelma, so app tabs initialize and get
  SSO tokens from kelma's auth broker,
- costs nothing while it is closed (kelma's idle baseline is about 30 MB).

Plan X3.1 named three candidates: CEF (cef-rs), a Chromium `--app` window, and wry/WebKitGTK.

## Options

| | Chromium `--app` + CDP | wry + WebKitGTK | CEF (cef-rs) |
|---|---|---|---|
| Engine | the user's Chromium-family browser | WebKitGTK 4.1 | bundled Chromium |
| Build needs | nothing (WebSocket client already in the tree) | `webkit2gtk-4.1-devel` headers (not installed here, needs root) | a ~150 MB CEF binary per platform |
| Runtime needs | Chromium, Chrome, Edge, Brave or Vivaldi on the system | WebKitGTK at runtime | none |
| Teams / M365 / ACS support | supported engine | WebKit is unsupported by Teams and ACS for media; fine for some tabs | supported engine |
| Memory while closed | 0 (separate process, started on demand) | the library stays mapped in kelma | the library stays mapped in kelma |
| Window | a separate top-level window (no tabs, no address bar) | can be embedded | can be embedded |
| Controlling the page | CDP: inject scripts, bindings, evaluate | native API | native API |

## Decision

A Chromium-family `--app` window driven over the Chrome DevTools Protocol, in the new crate
`crates/webhost` (`webhost::open(url, opts) -> WebWindow`).

- **Browser.** A configured path, else `$KELMA_BROWSER`, else the first of `chromium`,
  `chromium-browser`, `google-chrome(-stable)`, `microsoft-edge(-stable)`, `brave-browser`,
  `brave`, `vivaldi(-stable)` on `PATH`.
- **Launch.** `--app=data:text/html, --user-data-dir=<kelma data dir>/webhost-profile
  --remote-debugging-port=0 --no-first-run --no-default-browser-check`, in its own process
  group. The window starts on an empty `data:` page (Vivaldi turns `about:blank` into its start
  page); the app's URL is loaded only after the shim is registered, so it runs before the app's
  own scripts.
- **CDP.** kelma reads the port and browser path from `<profile>/DevToolsActivePort`, connects
  to the browser WebSocket, attaches to the page with a flattened session and then:
  - `Runtime.addBinding` for `__kelmaTeams` and `__kelmaBridge`; calls arrive as
    `Runtime.bindingCalled`, replies go back with `Runtime.evaluate`;
  - `Page.addScriptToEvaluateOnNewDocument` injects `crates/webhost/src/shim.js`;
  - `Target.targetDestroyed`, `Target.detachedFromTarget` and process exit mark the window
    closed (`WebWindow::closed()`).
- **TeamsJS shim.** A top-level page has no parent window, so teams-js 2.x uses its "frameless"
  transport: `window.nativeInterface.framelessPostMessage(json)` out and
  `window.onNativeMessage({ data })` in. The shim defines `nativeInterface` and answers
  `initialize` (frame context `content`, host client type `desktop`, a v4 runtime that claims
  only what the shim implements), `getContext`, `authentication.getAuthToken` (routed to a Rust
  `TokenCallback`), the `appInitialization.*` notifications, `executeDeepLink`, and theme
  changes (`WebWindow::set_theme`). Other calls get a "not supported" SDK error so app promises
  do not hang. Checked against teams-js 2.59.0 (`dist/umd/MicrosoftTeams.min.js`).
- **Origins.** Shim messages are accepted only from the opened URL's origin (or a configured
  list, for the app's `validDomains`). The origin comes from the CDP execution context, which
  the page cannot fake, so a page the app navigates to cannot ask kelma for tokens.
- **Lifetime.** One browser process per window. Closing the window ends the process;
  `WebWindow::close()` and dropping the `WebWindow` send SIGTERM to the process group (so the
  browser saves cookies) and SIGKILL after 5 s.

## Profile and sign-in

The profile is kelma's own (`<data dir>/webhost-profile`), never the user's main browser
profile. The user signs in once inside the host window; cookies persist there, so later windows
are already signed in (SSO). Recent Chrome versions refuse remote debugging on the default
profile anyway. Only one browser can hold a profile: `open` waits up to 5 s for a closing window
and then fails with `ProfileInUse`.

## Consequences

- No build dependency and no memory cost while closed. The user needs a Chromium-family browser
  (most desktops have one; Flatpak needs one reachable from the sandbox).
- The host is a separate window, not a pane inside kelma's window.
- Each window is its own browser process. Measured 2026-11-19: about 495 MB PSS for all Brave
  processes with a one-line page open (fresh profile). It is freed when the window closes.
  Sharing one process between windows (`Target.createTarget` in the running browser) is a later
  step.
- The DevTools port listens on 127.0.0.1, so other local processes could connect while a
  window is open. `--remote-debugging-pipe` (CDP over inherited file descriptors) would remove
  that and is the follow-up.
- App SSO: the shim hands `getAuthToken` to the caller. kelma's broker still has to mint a
  token for an app's own resource (`api://<domain>/<app id>`), which needs a per-resource
  audience in `teams_proto::broker`.

## WebKitGTK, if it is needed later

wry with WebKitGTK embeds the view in kelma's window and needs no browser, but it needs the
`webkit2gtk-4.1-devel` headers to build, WebKit is not supported by Teams and ACS for media,
and the library stays loaded while unused. It remains an option for simple tabs that need
embedding; the shim protocol code (`webhost::shim`) does not depend on CDP and would carry over.
