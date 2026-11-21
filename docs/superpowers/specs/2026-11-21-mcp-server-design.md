# MCP server for IronTeams: design

Date: 2026-11-21. Status: approved by the user and implemented on `feat/mcp-server`.

## Why

The goal is to let Claude (Claude Code or Claude Desktop) drive Teams through IronTeams, so that
everyday work gets more automated:
- "what did I miss";
- search;
- the calendar;
- replies and status.

`ironteams-cli` already has almost every operation and resumes the app's saved session, so an MCP
server over stdio is a thin layer. It needs no API key and no UI change.

## Decisions (made by the user)

- **Scope:** read, plus write with confirmation.
- **Location:** a subcommand, `ironteams-cli mcp`.
- **Accounts:** the server is off until `mcp allow <label>`. That command warns that what Claude
  reads goes to Anthropic. IronTeams signs in only to work and school accounts, so it cannot tell
  a "personal" account apart; the allowlist is how "personal only by default" is enforced.
- **Confirmation:** two layers.
  - Each write is a draft first, and only `send_draft` does it.
  - The client's own prompt before `send_draft`, made mandatory with a printed `ask` rule.
- **Read tools:** catch-up, search, calendar, people and presence.
- **Write tools:** send and reply, presence and status note, meeting RSVP, reaction and mark-read.
- **Conversations:** chats only. Channels are refused by every tool.
- **Audit:** writes only, without text.

## Approaches considered

1. **A subcommand of `ironteams-cli` (chosen).** It reuses `teams-proto`, `calendar` and
   `session`, and needs the least code.
2. **A separate `ironteams-mcp` crate.** It would be cleaner to package, but would duplicate the
   session and client setup.
3. **Inside the desktop app.** It could drive the open window, but needs a new IPC channel, which
   means much more work.

## Design

The modules live in `crates/ironteams-cli/src/mcp/`:

| Module | Contents |
|---|---|
| `mod.rs` | Commands, `serve`, and the rmcp `ServerHandler` with its instructions |
| `guard.rs` | The allowlist and the chat-only check |
| `drafts.rs` | Drafts: frozen `Action`, 10-minute life, single use, ids that a new process never repeats |
| `audit.rs` | The JSONL log |
| `text.rs` | Model-facing text |
| `read.rs` | The read tools |
| `write.rs` | The write tools |

The SDK is `rmcp` 3.11, the official Rust MCP SDK (Apache-2.0), with tool macros and the
stdio transport.

### Calls the tools reuse

| Use | Function |
|---|---|
| Chats and messages | `Client::conversations`, `messages` |
| Search | `search_messages` |
| Mentions | `mentions` |
| People and presence | `search_people`, `get_presence` |
| Sending | `send_message_as`, `send_quote_reply_as`, with the client message id fixed at draft time |
| Reactions | `add_reaction` |
| Mark read | `mark_read` |
| Status | `set_presence`, `reset_presence`, `set_status_note` |
| Calendar | `calendar::Calendar::view`, `event_detail`, `find_meeting_times`, `respond` |
| Message text | `render::parse_teams_html`, `plain_text`, `text_to_teams_html`, `reply_html` |

### Settled by the builder

- MCP elicitation is not used in v1. On MCP protocol 2026-07-28, server-to-client requests are
  replaced by input-required results, so the client's permission prompt is the second layer.
- Times are UTC.
- `calendar_agenda` covers now to now + N days.
- Titles of 1:1 chats are not resolved: they show "(no title)" with the last sender.

## Out of scope

- Channels.
- Files.
- Editing or deleting messages.
- Scheduled messages.
- Calls.
- Driving the desktop app.
