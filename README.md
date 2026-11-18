# kelma

A lightweight native desktop client for Microsoft Teams chat on Linux, written in Rust.

The Teams web app takes around 1 GB of RAM. kelma aims to provide the same chat experience
(chats, channels, reactions, replies, images and files) in about 100 MB.

> **Status: early development.** Nothing usable yet. See the roadmap below.

## Disclaimer

kelma is an **unofficial** project. Microsoft does not affiliate with it, endorse it or support it.
"Microsoft Teams" is a trademark of Microsoft Corporation.

kelma talks to the same internal endpoints the Teams web client uses. Those endpoints are
undocumented and Microsoft can change them at any time, which would break kelma. Your
organization's policies may not allow third-party clients. Check with your IT team before you
use it with a work account. Use it at your own risk.

Calls, meetings and screen sharing are not supported and are out of scope. Use the official
web app for them.

## Roadmap

- [ ] Phase 0: scaffold and UI toolkit spike (Slint vs iced, with a focus on Arabic/RTL text)
- [ ] Phase 1: protocol core and `kelma-cli` (login, chats, history, send, live updates)
- [ ] Phase 2: reactions, replies, edits, channels, files
- [ ] Phase 3: desktop UI MVP
- [ ] Phase 4: hardening, packaging

## Credits

The protocol knowledge comes from [purple-teams](https://github.com/EionRobb/purple-teams)
by Eion Robb (GPL-3.0).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
