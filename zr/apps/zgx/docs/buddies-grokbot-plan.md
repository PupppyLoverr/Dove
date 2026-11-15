# Buddies, rebuilt on Grok Bot's shape, plus Connections

Status: plan, nothing built. Written 2026-11-15.

## What we're matching

Grok Bot (xAI's, billed through Cursor; installed here at
`%LOCALAPPDATA%\Programs\Grok Bot`) is a roster of always-on bots. Sources
for this plan: its docs (docs.x.ai/grok-bot), the strings in its installed
app bundle, your 10 reference screenshots and its `~/.grokbot/settings.json`.

**Layout** (screenshots 2, 6 and 7)
- **Left:** a 3-column grid of pinned bots, then the rest in rows. Its footer
  holds your picture and a **Connect apps** pill showing the marks of
  connected apps.
- **Centre:** messaging-style chat.
  - A floating name pill at the top, with **Share**.
  - Your bubbles are tinted in the bot's colour.
  - Reply-to quotes, reactions, and pages shown as cards.
  - Composer: `+`, "Message <bot>", a mic, and a voice-chat button.
- **Right:** the bot's drawer.
  - Its face (click to edit), its name, and two tabs, **Activity** and
    **Library**.
  - Activity shows Suggested, Computer (with a monitor thumbnail) and Voice
    chat.
- **No settings page for a bot.** You configure a personal bot by chatting
  with it. Grok Bot has **Bot details** only for Team Bots.

**What a bot can do**

| Area | Grok Bot |
|---|---|
| Bots | Create (shape, colour, generated or uploaded picture); a **primary Bot** that checks in and coordinates the others; pin, sections, hide, mark unread, rename, duplicate, delete; share as a template; a template gallery |
| Chat | Reply, react, threads, attachments, dictation, steer or **Stop now** mid-turn, `@` bots/routines/apps, **drafts** (email and Slack wait for **Send**), voice memos, find in chat, command palette (Ctrl+K), Search Bots (Ctrl+Shift+F) |
| Group chat | 2–6 bots in one chat, `@everyone`; bot-to-bot handoffs |
| Memory | Per bot, plus personal facts shared by all bots; "remember", "forget"; **import memories from another assistant** |
| Skills | A shared private library, enabled per bot; "save this as a skill"; **Teach a task** (record a demo on its computer) |
| Routines | Plain-language schedules; triggers on Slack messages, email to the bot, webhook (POST with a bearer key), GitHub/Linear/Sentry events; pause, resume, **Test**, edit, delete; last 20 runs; at most 50 per bot, 5 minutes apart |
| Computer | One persistent cloud computer shared by your bots (browser, terminal, `/workspace`); a live view; **take over**; sign-in handoff; Update/Recover/Reset; local-computer policy **Ask every time / Always allow / Never allow** |
| Connect apps | A catalog with categories, OAuth, multiple accounts, **custom MCP connectors** (URL plus headers, or command), per-app approval rules **Allow without asking / Always ask**, custom rules, **Auto-review** levels |
| Vault | Logins, cards, API keys; secure forms the bot never sees |
| Channels | Email (its own address, Gmail/Outlook drafts), Slack, Teams, X, iMessage (Mac), phone calls (US) |
| Other | Voice chat with voice/speed/language; notifications when a bot finishes or waits; teams and Team Bots; shopping and booking |

## Our limits

- **Bring your own key.** Bots think through your own harnesses (Claude Code,
  Codex and the others Zeron drives) on your own logins.
- **Zero budget.** No servers of our own. Everything runs on this PC, in a
  free local Cua Space, on your own Boat account, or through your own
  Composio account (free plan: about 10,000 app tool calls a month, more
  with your own OAuth apps; check composio.dev/pricing).
- **So these are out of scope:**
  - Phone calls and SMS.
  - Shopping and booking.
  - Teams and Team Bots.
  - X Chat.
  - iMessage (we're on Windows).
  - Running while this PC is off, unless the bot's computer is on Boat.

## Part 1: Connections (Composio), a global settings page

### How Composio connects
- **Endpoint:** `https://connect.composio.dev/mcp`, streamable HTTP.
- **Auth:** your consumer key `ck_…` in the header `x-consumer-api-key`
  (from the dashboard's AI Clients page). Without a key it falls back to
  MCP OAuth in the browser.
- **Tools:** it exposes 7 meta-tools rather than one tool per app:
  - `COMPOSIO_SEARCH_TOOLS`
  - `COMPOSIO_GET_TOOL_SCHEMAS`
  - `COMPOSIO_MULTI_EXECUTE_TOOL`
  - `COMPOSIO_MANAGE_CONNECTIONS`
  - `COMPOSIO_WAIT_FOR_CONNECTIONS`
  - `COMPOSIO_REMOTE_WORKBENCH`
  - `COMPOSIO_REMOTE_BASH_TOOL`
- **Connecting an app:** gives a short-lived OAuth link. Connections persist
  on your Composio account.

### The gap in our engine
`zeron_proto::McpServer` (`crates/proto/src/agent.rs:647`) only describes a
local command: `command`, `args` and `env`. It has no URL and no headers.
The engine hands a chat its servers in `sessions.rs:3562`: Zeron's own
`zeron mcp`, plus plugins' native servers.

### Design
1. **A bridge, not a URL.**
   - Add an engine subcommand, `keplorayx-engine mcp-composio`. It is a stdio
     MCP server that relays to Composio over streamable HTTP and adds the
     key header itself.
   - It works with every harness, because all of them take stdio servers.
   - The key never reaches a harness's config or argv.
   - The engine adds it beside `zeron mcp` in `sessions.rs`, with the same
     env pattern (`ZERON_CHAT_ID`, the IPC token file).
2. **The key.** Store it sealed with DPAPI, like `kx/keys.rs`. The sealing
   moves to a small shared module so the engine binary can open it.
   - Settings: a single **Composio key** field with **Get a key ↗**, which
     opens the AI Clients page.
   - Or **Sign in with Composio** (MCP OAuth with PKCE, run by the bridge,
     tokens sealed the same way). This is phase 1b.
3. **Scope, enforced in two places.**
   - **Policy file.** `<data>/connections.json`:
     `{ composio: { on, default_scope, apps: { gmail: { scope, buddies: [ids] } } } }`.
     A scope is one of:
     - `everywhere` (the default)
     - `zeron` (ordinary chats only)
     - `buddies` (every bot)
     - `some` (the bots listed)
   - **Buddy chats.** The app writes the chat id → buddy id map beside the
     policy whenever buddies change, so the engine never reads Buddies files.
   - **Engine.** It doesn't attach the server at all to a chat that no
     connection reaches.
   - **Bridge.** In a chat, it filters by app (a tool slug's prefix, e.g.
     `GMAIL_…`):
     - It drops tools from `SEARCH_TOOLS` results that the chat can't use.
     - It refuses `MULTI_EXECUTE_TOOL` items for such apps.
     - It shows only the allowed apps in `MANAGE_CONNECTIONS`.
4. **Approvals at the bridge** (Grok Bot's Allow without asking / Always ask).
   - Each app gets a rule:
     - **Ask before changes** (the default)
     - **Allow without asking**
     - **Always ask**
   - Composio marks some tools as read-only. For the rest, the bridge holds
     the call and raises an approval card in the chat over the engine's IPC.
     The call goes ahead only on Allow.
   - A per-bot override sits in the bot's drawer.
5. **Settings → Connections page.** New file
   `ui/screens/settings/connections.luau`, listed under Personal.
   - **Header:** status (Connected as …, N apps) and the key field or
     sign-in.
   - **Connected apps:** one row per app.
     - Its logo, its name, and the account it's connected as.
     - A **Used by** control: Everywhere · Zeron · All bots · Some bots.
       Some bots opens a picker of buddy faces.
     - An **Approvals** control.
     - **Disconnect.**
   - **Add an app:** search the catalog; the categories are chips. **Connect**
     opens the OAuth link in the browser, waits with
     `COMPOSIO_WAIT_FOR_CONNECTIONS`, then adds the row.
   - The same component is the **Connect apps** dialog in Buddies (the
     sidebar pill and the in-chat Connect card).
   - The look follows screenshot 5: two columns, quiet rows, no dividers.
6. **Catalog source.** The app list and logos come from Composio. Still to
   check: whether a toolkit listing works with a consumer key, or only
   through `COMPOSIO_SEARCH_TOOLS`. If only the latter, show a curated list
   of about 40 common apps with a "search all" box.

**Done when:**
- With a `ck_` key, a Claude Code chat and a buddy can each list and use
  Gmail.
- A buddy outside Gmail's scope can't see Gmail's tools.
- A send waits for Allow.
- Tests: the bridge's filtering and approvals, with a fake Composio server.

## Part 2: Buddies, redesigned on Grok Bot's layout, and smaller

Buddies UI today is 4,421 lines in 7 files (`page.luau` alone is 1,899).
Grok Bot has no per-bot settings page, so neither will we.

| Today | Becomes |
|---|---|
| Board page (`M.board`) | Gone. An empty state with templates when there are no buddies; otherwise you land in the last bot's chat |
| Buddy page with 5 tabs | Gone. Its settings move into the drawer |
| Drawer (Activity, Library) | **Activity · Library · Settings**. Settings is compact: name, role, instructions, model order, folder, permissions, computer, connections used, approvals |
| Sidebar | Grok Bot's: 3-column pinned grid, rows, **sections** (drag to move), Hidden bots, a footer with you and **Connect apps** (connected marks) |
| Chat bar | A floating centre pill (face and name; click opens the drawer) and **Share** (export the bot as a template file) |
| Chat | Your bubbles tinted in the bot's colour (a setting, like Grok Bot's `botColorInChat`); page cards for files it writes; draft cards |
| Create | The Create New Bot dialog as now; the picture popover gets **Bot · Upload · Reset** (Upload takes a local PNG, JPEG or WebP; no paid image generation) |
| Palette | A **Bots** section in Ctrl+K with faces and last lines, Ctrl+1…6, New bot |

**Size target:** 7 files and about 2,400 lines.
- `store.luau`
- `avatar.luau`
- `sidebar.luau`
- `chat_bar.luau`, which takes over the pill and drawer frame
- `drawer.luau`, new: the Activity, Library and Settings tabs, built from
  what's kept of `page.luau`
- `runner.luau`
- `templates.luau`

`page.luau` is deleted.

## Part 3: Doing what Grok Bot does, within our limits

| Feature | How, at zero cost |
|---|---|
| Memory | `MEMORY.md` in each bot's home, plus a shared `ABOUT-YOU.md` all bots read; its instructions say when to remember and forget. Import: the Grok Bot import exists; add pasting memories from another assistant. The drawer's Library lists both |
| Skills | A shared folder `<data>/skills/<name>/SKILL.md`, enabled per bot (written into its home where its harness looks, e.g. `.claude/skills`). "Save this as a skill" is an instruction. Composio's apps count as skills in Connect apps |
| Primary bot | A flag. It gets a short daily check-in routine and a tool to message other bots |
| Bot-to-bot | A `message_buddy` tool in `zeron mcp` (it already starts side chats): one bot writes to another's chat and waits for its reply |
| Group chat | Later: a chat with 2–6 bots, each answering when `@`'d |
| Routines | Have: schedules. Add: **Test** (run now), run history (20 per routine), a pause switch on the row, a **webhook** trigger (POST to a local port with a bearer key, while Keplorayx runs), and **poll** triggers through Composio (new Gmail or Slack message every N minutes, deduped). At most 50 per bot, 5 minutes apart |
| Drafts | Sending tools through Composio go through approvals (Part 1.4); the card shows the draft with Send and Discard |
| Computer | Have: a Cua Space or Boat per bot, and the desktop beside its chat. Add: wake it when a run needs it; **Take over** (the viewer, with input); one shared Space for all bots as an option (as in Grok Bot); local-command policy Ask every time / Always / Never, mapped to the harness's permission mode |
| Vault | Per-bot secrets sealed with DPAPI, passed to its harness as environment variables; a masked secret card in chat |
| Notifications | A Windows toast when a bot finishes or waits on you (per bot, on by default) |
| Templates | Share: export the bot (look, job, skills, routines; no history or memory) as a `.kxbot` file; Import |
| Voice | Dictation exists. Voice chat (Windows speech in and out, free) is a stretch goal, last |

## Part 4: Fast

Measure first, on the scratch app with 6 buddies: idle CPU, how long the
sidebar takes to show, and frame time while one bot works. Note the app
already idles at about 200% CPU without Buddies, so that is looked into
separately.

- **One clock for every face.** One `ui.every` drives every avatar, instead
  of a blink timer and a 50 ms tick each. Idle faces don't tick.
- **One status map.** Compute buddy status once per change of
  `sessions.now`, as a map by chat. Today `store.standing` scans every chat,
  for every face, on every draw.
- **Cached outlines.** Cache each body outline by shape and size; today
  every redraw rebuilds a 96-point path.
- **Cursor redraws.** Only faces within reach of the cursor redraw on its
  moves.
- **Lazy panels.** Nothing in a closed drawer tab is built. The Spaces list
  is cached (it shells out to `cua`, which takes seconds) and refreshed in
  the background, never on open.
- **Keyed lists.** Every `ui.each` gets a stable key, so lists aren't
  rebuilt.

## Order of work

1. **Hand-off.** The session "btw: i am awake bro what's happening" has
   asked for no edits to `ui/kx/buddies/*` until it's done. Get its
   go-ahead, or have it take Part 2.
2. **Part 1** (Connections). It touches no Buddies files: the engine
   bridge, the policy file, the settings page and tests.
3. **Part 4's measurements and fixes**, then **Part 2** (the redesign and
   shrink), with before/after screenshots.
4. **Part 3**, in this order:
   - routines (Test, history, webhook)
   - approvals and drafts
   - memory and skills
   - notifications
   - bot-to-bot and the primary bot
   - templates
   - computer wake and take over
   - vault
   - voice
5. Each part is committed on its own, with your OK. Nothing is pushed.

## Progress (2026-11-15)

**Built, with tests passing:**
- **The Composio bridge** (`crates/composio`, 13 tests): scope, approvals by
  `confirmed`, sandboxes hidden when restricted.
- **Engine wiring:** the `mcp-composio` command and the engine attaching it
  to chats in scope.
- **Connections in Settings:** the app side (`kx/connections.rs`,
  `settings/connections.luau`), built by the plan session.
- **Back end in `kx/buddies.rs`** (30 kx tests pass):
  - Memory: `MEMORY.md` per buddy and a shared `buddies/ABOUT-YOU.md`.
  - A skills library (`<data>/skills`), per-buddy `skills`, mirrored to
    `.claude/skills`.
  - Each buddy's instructions name its teammates and their chats, to hand
    work to over Zeron's `send_message`.
  - Templates: `kx.buddy.export` / `kx.buddy.import_template`
    (`.kxbot`; routines come in off).
  - Webhook routines: `webhook: true` →
    `POST 127.0.0.1:47821/routines/<id>` with a bearer key.
  - A shared computer (`kx.computer.shared/share`, `computer.kind =
    "shared"`, the default for new buddies once one is set).

**Waiting on the Buddies redesign** (its files are another session's for now):
- The UI for skills, templates, webhooks and the shared computer.
- The Grok Bot layout, faces and size cut.
- Notifications when a bot finishes or waits.

**Not started:**
- An Allow card in the chat, in place of `confirmed`.
- Composio sign-in through the browser.
- Poll triggers.
- Per-buddy vault.
- Voice.

## Decided (2026-11-15)

1. **Faces:** match Grok Bot: tilted pill eyes, its 13 shapes and 11
   colours, `^^` when done. The cursor-following stays.
2. **The buddy page:** gone, along with the board. Settings move to the
   drawer's Settings tab.
3. **Bot computers:** one shared Space for all bots by default; a bot can be
   given its own.
4. **App approvals by default:** ask before changes. Reading runs freely;
   anything that sends, posts, deletes or changes waits for Allow.
5. **Composio sign-in:** the `ck_` key first; browser sign-in is a later
   step.
6. **Who does what:** this session builds Part 1 (Connections) now; it
   touches no Buddies files. The redesign (Parts 2–4) starts from the other
   session's result once it's done.
7. **Voice chat:** last.
8. **Commits:** one per finished, tested part, only this session's files,
   never pushed.
