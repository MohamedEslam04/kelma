# kelma

A lightweight native desktop client for Microsoft Teams chat on Linux, written in Rust with [Slint](https://slint.dev).

The Teams web app takes around 1 GB of RAM. kelma gives you the same chat experience in a familiar
Teams-like layout and uses a fraction of that memory: see [Memory](#memory).

![Chat](docs/screenshots/chat.png)

> **Unofficial.** Microsoft does not affiliate with, endorse or support kelma. "Microsoft Teams"
> is a trademark of Microsoft Corporation. See the [disclaimer](#disclaimer).

## Features

- **Sign-in:** sign in with a device code (works with MFA). The refresh token lives in your system
  keyring (KWallet or GNOME Keyring), never in a file.
- **Chat list:**
  - 1:1, group, meeting and self-notes chats
  - Favorites, Chats and "Teams and channels" sections
  - Unread / Channels / Chats filters and search
  - real profile photos
- **Teams and channels:**
  - read posts with their subject lines and reply counts
  - open threads and reply in them
  - start new posts
- **Messages:**
  - rich text: bold, italic, links, lists, code, @mentions, reply quotes, edited and deleted markers, day separators
  - Arabic, English and mixed-direction text, each paragraph laid out in its own direction
  - inline images
  - file cards (open in browser)
  - voice messages and videos (play in your system player)
  - bot and workflow cards (Adaptive Cards)
  - color emoji
- **Actions:**
  - send and reply
  - react, edit, delete
  - paste emoji
  - upload images
  - send files: uploaded to your OneDrive and shared with the chat, as Teams does (chats only, not channels)
  - typing indicator
- **Live:** real-time updates over Teams' push channel (new messages, edits, reactions, typing),
  automatic reconnect and catch-up, and desktop notifications when the window is not focused.

Calls, meetings and screen sharing are **out of scope**. Use the Teams web app for them.

## Install

You need Rust (via [rustup](https://rustup.rs)) and the development headers for fontconfig,
xkbcommon and D-Bus.

```sh
# Fedora: sudo dnf install fontconfig-devel libxkbcommon-devel dbus-devel
# Debian/Ubuntu: sudo apt install libfontconfig1-dev libxkbcommon-dev libdbus-1-dev pkg-config
git clone https://github.com/MohamedEslam04/kelma && cd kelma
./packaging/install.sh      # builds and installs to ~/.local (binary, .desktop file, icon)
```

Start **kelma** from your launcher. On first start it shows a code: open the link, enter the code
and sign in with your work account.

### Environment variables

| Variable | Effect |
|---|---|
| `KELMA_RENDERER=femtovg` | GPU renderer instead of the default software renderer (uses more memory) |
| `KELMA_LOG=kelma=info,teams_proto=info` | Logging to stderr. Tokens are never logged |
| `KELMA_DEMO=1` | Offline demo with sample data: no account, no network |
| `KELMA_HOME=<dir>` | Keep config, cache and data under `<dir>` |

## Memory

Measured from `/proc/<pid>/smaps_rollup` (PSS, which counts shared memory only once, plus
swapped-out pages where noted), release build, KDE Plasma on Wayland, Fedora 44:

| Client | Memory |
|---|---|
| Teams PWA in a Chromium browser | ~1100 MB |
| Pidgin + purple-teams | ~147 MB |
| **kelma**, real work account, chat with images open | **~62 MB** (PSS 8 + swapped 54) |
| **kelma**, real work account, 10 min idle, chat list only | **~29 MB** PSS (swap not counted) |
| kelma demo mode | 39 MB |

kelma keeps the conversation list and messages in an SQLite cache. Only the visible part of the
message list exists in memory (virtualized list), and images are downscaled when they are decoded.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
KELMA_DEMO=1 cargo run -p kelma
cargo run -p kelma-cli -- login | chats | teams | read <id> | send <id> <text> | tail
```

| Crate | Role |
|---|---|
| `crates/teams-proto` | Protocol: auth, chat service, channels, profiles, AMS media, trouter real-time, cards |
| `crates/render` | Teams HTML to a rich-text model, with per-paragraph text direction |
| `crates/store` | SQLite cache |
| `crates/session` | Keyring and config persistence |
| `crates/kelma` | The desktop app (Slint UI and engine) |
| `crates/kelma-cli` | Command-line client for protocol work |

[`docs/protocol.md`](docs/protocol.md) documents every endpoint and what has been verified.
[`docs/adr/0001-ui-toolkit.md`](docs/adr/0001-ui-toolkit.md) explains the toolkit choice.

## Disclaimer

kelma talks to the same internal endpoints the Teams web client uses. They are undocumented and
Microsoft can change them at any time, which would break kelma. Your organization's policies may
not allow third-party clients, so check with your IT team before you use kelma with a work account.
Use it at your own risk.

## Credits

- Protocol knowledge from [purple-teams](https://github.com/EionRobb/purple-teams) by Eion Robb (GPL-3.0).
- Color emoji: [Twemoji](https://github.com/mozilla/twemoji-colr) (CC-BY 4.0). See `crates/kelma/ui/fonts/README.md`.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
