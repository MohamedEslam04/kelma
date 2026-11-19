# kelma plan: Copilot

Part of the Teams parity plans (see `plans/README.md`). Format as in `plans/chat.md`.

**Sources.**
- UI survey of the Copilot app in Teams web (2026-11-18): New chat, Agents, Notebooks, chat history, the "Auto" model picker, "+" attach, the enterprise data protection shield, and prompt chips (Refine writing, Draft content, Summarize, Find content, …).
- `docs/research/apps-onedrive-copilot-store.md`, including community research on the protocol.
- The token probe: **`https://substrate.office.com/sydney` mints**, with CopilotPlatform* scopes.

**Licensing.** Copilot Chat works for work accounts without a paid license: web-grounded, with
enterprise data protection. Grounding on mail, files and chats needs Microsoft 365 Copilot; the
server enforces this.

**Approach.** Native, over the same WebSocket the Teams client uses. It is undocumented, so every
item has a fallback: open `m365.cloud.microsoft/chat` in the web host.

## Phase P0: discovery

- [ ] **P0.1** DEFERRED: needs test tenant (Copilot is not allowed on the work account). Capture the Teams Copilot app traffic and document it in `docs/protocol-copilot.md`:
  - the WebSocket `wss://substrate.office.com/m365chat/SecuredChathub/{oid}@{tid}?…&source=teamshub` (SignalR JSON frames separated by `0x1E`, invocation `target:"chat"`, streamed partial and final messages with `adaptiveCards`, `sourceAttributions` and `suggestedResponses`)
  - history, agents (`/search/api/v1/userconfig` "CopilotPlugins"), notebooks, file upload

## Phase P1: chat

- [ ] **P1.1 Copilot chat.** DEFERRED: needs test tenant (Copilot is not allowed on the work account). Impact 4, effort 4.
  - **What.**
    - A Copilot rail item.
    - "Message Copilot" box, with answers streaming in as Markdown with citations (numbered chips that open the source) and suggested follow-ups as chips.
    - Prompt chips on an empty chat.
    - Stop generating, Copy, thumbs up and down, Regenerate.
  - **How.** A `copilot` module in `teams-proto` on tokio-tungstenite (the same pattern as trouter).
  - **Done when.** A question gets a streamed answer with citations, matching Teams.
- [ ] **P1.2 History.** DEFERRED: needs test tenant (Copilot is not allowed on the work account). New chat, the list of past chats (pin, rename, delete), reopening a chat.
- [ ] **P1.3 Attach.** DEFERRED: needs test tenant (Copilot is not allowed on the work account). "+" attaches a file from OneDrive or the device, or an image.
- [ ] **P1.4 Model and mode.** DEFERRED: needs test tenant (Copilot is not allowed on the work account). The "Auto" picker (whatever the server offers) and the work/web state shown with the shield.

## Phase P2: more

- [ ] **P2.1 Agents:** DEFERRED: needs test tenant (Copilot is not allowed on the work account). list and open agents and chat with them.
- [ ] **P2.2 Notebooks** DEFERRED: needs test tenant (Copilot is not allowed on the work account). (Copilot Pages): list and open; editing in the web host.
- [ ] **P2.3 Copilot in chats and meetings:** DEFERRED: needs test tenant (Copilot is not allowed on the work account). "Summarize this chat" and "Catch up". This is license-gated and needs discovery (the in-chat Copilot uses the same hub with a chat context).

## Fallback

- [ ] **P9.1** DEFERRED: needs test tenant (Copilot is not allowed on the work account). If the WebSocket route breaks or is refused, the Copilot rail item opens `https://m365.cloud.microsoft/chat` in the web host, signed in once.

## Order

P0, then P1.1 and P1.2, then the rest.
