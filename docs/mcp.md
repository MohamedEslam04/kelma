# Using IronTeams from Claude (MCP)

`ironteams-cli mcp serve` is a [Model Context Protocol](https://modelcontextprotocol.io) server. An
MCP client such as Claude Code or Claude Desktop starts it and can then do the following on one
signed-in IronTeams account:

- read your chats, calendar and people;
- draft messages, reactions, status changes and meeting answers, which happen only after you
  approve them.

It runs on stdio, uses the session the desktop app already saved in the keyring, and needs no API
key.

**It is not in release builds.** The server is behind the `mcp` cargo feature, which is off by
default. Build it yourself to use it:

```sh
cargo build --release -p ironteams-cli --features mcp
```

Without the feature, `ironteams-cli mcp` only says that the build has no MCP server.

## Before you start: where your data goes

Whatever the client reads goes to its AI provider: messages, names, meeting text. For Claude, that
provider is Anthropic. For a work account, check that your organization allows this first. Because
of this, the server is **off for every account** until you turn it on:

```sh
ironteams-cli accounts                    # find the account label
ironteams-cli mcp allow <label>           # turn it on for that account (prints this warning)
ironteams-cli mcp deny <label>            # turn it off again
ironteams-cli mcp status                  # allowed accounts, allowlist and audit log paths
```

The allowlist is `mcp.json` in the config directory (next to `config.json`). The server runs on
exactly the account named when it starts, and never changes the account the app has active.

## Setup

```sh
ironteams-cli mcp setup --account <label>
```

This prints three things. It edits nothing.

- The Claude Code command:

  ```sh
  claude mcp add --scope user ironteams -- /path/to/ironteams-cli mcp serve --account <label>
  ```

- A permission rule that makes Claude Code ask you before every send. Add it to
  `~/.claude/settings.json` and don't answer "always allow" for `send_draft`:

  ```json
  { "permissions": { "ask": ["mcp__ironteams__send_draft"] } }
  ```

- The `mcpServers` entry for Claude Desktop's `claude_desktop_config.json`.

## Tools

Every tool works on **chats only**: 1:1 chats, group chats, meeting chats and your notes
(`48:notes`). Team channels are refused everywhere. Times are UTC.

| Tool | What it does |
|---|---|
| `list_chats` | Chats with id, kind, title, read state and last message, optionally unread only |
| `read_chat` | The messages of one chat, with message and sender ids; pages back with `before_ms` |
| `catch_up` | Unread chats with their unread messages, plus the chat messages that mention you |
| `search_messages` | Message search, optionally in one chat or from one person |
| `find_people` | People by name or email, with presence and status message |
| `get_presence` | Presence of people by id |
| `calendar_agenda` | Your events for a range of days |
| `meeting_detail` | One event: attendees, responses, invitation text |
| `find_free_time` | Free slots for you and the given attendees |
| `draft_message` | Drafts a message or a quoted reply |
| `draft_reaction` | Drafts a reaction (like, heart, laugh, surprised, sad, angry) |
| `draft_mark_read` | Drafts marking a chat read |
| `draft_presence` | Drafts your status for some minutes, or a reset to automatic |
| `draft_status_note` | Drafts your status message (empty clears it) |
| `draft_meeting_response` | Drafts accept, tentative or decline; the organizer gets it |
| `send_draft` | Does one approved draft |
| `discard_draft` | Throws a draft away |

## How a write happens

1. A `draft_*` tool freezes exactly what would happen and returns a draft id and a preview: the
   chat (kind, title, member count) and the exact text or change. Nothing is sent.
2. Claude shows you the preview. If you approve, it calls `send_draft` with the id, and Claude Code
   asks you once more because of the `ask` rule.
3. `send_draft` does the frozen action. It never uses new parameters from the call. A draft can
   be sent once, within 10 minutes. Drafts live only in the running server.

Every send attempt adds a line to `mcp-audit.jsonl` in the data directory (`~/.local/share/ironteams/`):
the time, account, kind of write, chat or event id, draft id and outcome (`sent`, `failed`,
`expired`, `unknown_draft`, `refused`). The text is never logged.

Messages, names and meeting text come from other people. The server tells the client to treat them
as data, not instructions, and nothing is written without your approval.

## Not in this version

- Channels.
- Files and images.
- Editing or deleting messages.
- Scheduled messages.
- Calls.
- Controlling the running desktop app.
