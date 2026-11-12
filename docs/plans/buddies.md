# Buddies: a team of bots in Keplorayx

Plan, 2026-11-13. Nothing here is built yet.

## What it is

A **buddy** is a named bot with a job: standing instructions, a model to run
on, optionally a computer of its own, and optionally a schedule. You talk to
it like a chat, it keeps working between your messages, and you can watch its
desktop while it does. The idea follows Grok Bot and OpenMausBot (Apache 2.0;
ideas only, no code taken), built on what Keplorayx already has.

## What we already have, and what's missing

| Need | Today | Gap |
|---|---|---|
| Run any model | Chats on Claude Code, Codex, Cursor, Devin, Hermes and others, chosen per chat | No fallback when one hits its limit |
| Standing instructions | None: the engine's chat has no instructions field | Buddies store their own and send them |
| Schedules | None in the engine or app | A scheduler |
| A computer per bot | Sandbox tooling is installed (local and cloud machines, with a remote-desktop view) but not wired into the app | Create, attach, show |
| Watch and steer | Browser control with numbered cursors, a Stop button, approvals in chat, Kanban | A desktop view in the right panel |
| API keys | Each provider's sign-in | A Keys section, as a fallback when a subscription runs out |
| Away from the desk | Account sync of sessions | Out of scope for now |

## How it looks

### The Buddies tab

One new tab in the left sidebar, **Buddies**, between Search and Pull requests.
Opening it switches the window into the Buddies space:

- **The sidebar changes to the buddies' own.** Not the session list: a roster.
  Each row is a buddy's avatar and name, a status dot (working, waiting on you,
  idle, asleep, failed), and its model's mark. Above the roster: New buddy and
  Search. Below: Automations (the calendar of what runs when) and a way back
  to chats. It uses the same row sizes and tokens as the chat sidebar, with a
  tinted head so it is plain which space you are in.
- **The page** is the selected buddy:
  - **Chat**: the conversation with it (the existing chat page, unchanged).
  - **Profile**: name, avatar, job description, model order, working folder,
    computer, what it may do without asking.
  - **Automations**: its schedules and triggers, with last and next run.
  - **Activity**: what it did, newest first, each entry linking to the turn.
- **With no buddy selected**: the team board. A card per buddy showing what it
  is doing now, and starter templates (chief of staff, support triage,
  bookkeeper, researcher, code reviewer) to create one from.

### The right dock panel for a buddy

Beside a buddy's chat the right panel opens with its own set of tabs, styled
as the buddy's (its colour on the active tab):

- **Desktop**: the live view of the buddy's computer. View only by default;
  a Take over button lets you click and type, and hands back when released.
  Pause and Stop sit in its bar. With no computer attached it offers to make
  one.
- **Browser**: the in-app browser the buddy drives (exists).
- **Files**: the buddy's working folder (exists).
- **Terminal**: a shell on the buddy's computer (exists for local).
- **Runs**: its automation runs and their results.

The panel can already be split into panes, so Desktop can sit beside the chat
while Files sits below it.

## How it works underneath

- **A buddy is a record plus a long-lived chat.** Records live in
  `~/.keplorayx/buddies.json`: id, name, avatar, instructions, model order,
  folder, computer, permissions, chat id. Scripts read them through one shared
  value, as widgets and the calendar do.
- **Instructions.** The engine has no system-prompt field, so the buddy's
  instructions are sent as the opening message of its chat and again, briefly,
  when a run starts after a long gap. If the engine gains a field, they move
  there.
- **Model order and fallback.** A buddy holds an ordered list (for example
  Claude Code, then Codex, then an API key). Before a run the app checks each
  one's standing; one that is at its limit or signed out is skipped, and the
  chat says which was used and why. A fallback mid-turn starts a fresh turn on
  the next model with a short summary of where it stood.
- **Keys.** Settings › Keys holds provider API keys, write-only once saved,
  kept where the app keeps its other secrets. You enter them yourself.
- **Scheduler.** A small service in the app: schedules (every N minutes, daily
  at a time, weekdays, cron) and one-off runs, kept in
  `~/.keplorayx/automations.json`. Each run sends the automation's prompt to
  the buddy's chat. It runs only while the app is open; missed runs are listed,
  not replayed, unless the automation says to catch up.
- **Computers.** A buddy's computer is a sandbox made through the installed
  sandbox tooling: local first (free), cloud as an option. One per buddy, never
  shared, so one buddy's logins and files are not another's. The Desktop tab
  shows its remote-desktop stream in a web view.
- **Tools for the buddy.** Its agent gets the engine's existing tools plus
  computer tools scoped to its own sandbox, and the browser tools built this
  week. Anything on the "ask first" list (send, post, buy, delete) waits for
  approval in chat and in the island.
- **Buddies' own tools.** `list_buddies`, `message_buddy`, `buddy_status`, so
  one buddy (or any chat) can hand work to another. One level deep, as
  subagents are.

## Phases

Each phase ends with something usable and is checked on the running app.

1. **Buddies exist.** The tab, the roster sidebar, the team board, create and
   edit, templates, a buddy's chat with its instructions. Any installed model.
   *Done when:* you can make a buddy from a template, message it, and find it
   again after a restart.
2. **Fallback.** Model order, skipping a model at its limit, the note in chat.
   *Done when:* with the first model signed out or limited, a message still
   gets answered by the second, and the chat says so.
3. **Automations.** The scheduler, the Automations tab and calendar, run
   history, written from a sentence ("every weekday at 9, summarise my open
   pull requests"). *Done when:* a one-minute test schedule fires, the result
   is in Runs, and a restart keeps the schedule.
4. **Desktop, local.** A cua Space per buddy, the Desktop tab with live view,
   take over, pause and stop; computer tools scoped to that Space. *Done
   when:* a buddy opens an app in its Space and you watch it happen in the
   panel.
5. **Keys, and cloud computers.** Settings › Keys (provider API keys as the
   last step of the model order, and the Boat key); the Boat computers plugin.
   *Done when:* with a Boat key saved, a buddy gets a cloud desktop and the
   same Desktop tab shows it.
6. **Always on.** The engine on a machine that stays up, so schedules fire
   with the PC asleep.
7. **Later.** Connected apps, teach-by-recording, voice, phone, reaching a
   buddy by message from outside the app.

## Risks and things to decide

- **Credits.** Schedules spend model usage unattended. Each automation needs a
  run cap and the board should show what ran overnight.
- **Unattended actions.** A buddy with a computer and logins can do real
  damage. Default permissions are read and draft; sending, posting, paying and
  deleting ask first, and that default is per buddy.
- **Sandbox availability.** Local sandboxes need virtualisation on this PC; if
  it is not available, phase 4 starts with cloud ones instead.
- **Where the scheduler lives.** In the app it stops when the app closes; in
  the engine it survives. Start in the app, move it for phase 6.
- **The active template.** The user's layout (Zeron v2) carries its own copies
  of the sidebar, page and panel files; every phase touching those is done in
  both copies.
- **Naming.** "Buddies" in the app; "buddy" in code and data, not "bot", so it
  does not collide with existing agent and subagent words.

## Decided (2026-11-13)

- **Buddy chats appear only under Buddies**, never in the normal session list.
- **Templates in the first version:** code reviewer, researcher, chief of
  staff, support / inbox triage. (Triage is limited to drafting until
  connected apps exist.)
- **Permissions default to read and draft.** Sending, posting, paying and
  deleting wait for approval.
- **Computers, two ways:**
  - **Local:** a sandbox on this PC through cua Spaces (the tooling already
    installed). Free; stops when the PC sleeps.
  - **Cloud:** a remote desktop from Boat (boat.dev), the service OpenMausBot
    uses, by a Boat API key the user pastes into Settings › Keys. It ships as
    a Keplorayx plugin ("Boat computers"), so the app itself carries no
    provider: with the plugin on and a key saved, a buddy's Computer choice
    gains "Cloud (Boat)". Boat is paid after its trial, so the buddy's profile
    says a cloud computer may cost money before one is made.
  - Both show in the same Desktop tab. To check before phase 4: how each one
    exposes its screen (stream address or remote-desktop protocol), Boat's
    API and prices, and whether cua's local sandboxes run on this PC.

## Still open

- Is the Buddies space a separate view of the window (its own sidebar, as
  above), or a section inside the normal sidebar? This plan assumes separate.

## Where it stands (2026-11-13, after the first night)

Built and seen working in the app:

- Phases 1 to 3: the Buddies tab with its own sidebar, the team page and the
  four starters, a buddy's profile (job, face, the order of agents it runs
  on, folder, what needs a yes), its chat under Buddies only, routines by
  the clock with Run now, and the record of runs. One real run was sent and
  answered.

Built, checked by unit tests and in the page's tree, not yet used for real:

- Phase 4, a computer on this PC through cua: make, start, stop, remove,
  and the desktop in the panel. On this PC it could not be completed: cua
  has only QEMU here (no container engine), and the virtual machine stops
  at boot ("WHPX: Unexpected VP exit code 4"). A make that fails now cleans
  up after itself. It needs a PC where `cua sandbox create` comes up.
- Phase 5, a computer in the cloud on Boat: the key (sealed with Windows'
  DPAPI in `keys.json`, never given back to a script), make, start, stop,
  remove, and the desktop's address asked for afresh each time. No call to
  Boat has been made with a real key: the request and answer shapes are
  from Boat's published API pages.

Not built:

- Keys for model providers as a fallback when every signed-in agent is at
  its limit: the engine has no way to run a turn on a bare API key, so a
  key field would do nothing.
- A panel of its own style for a buddy's chat (the desktop opens as a
  browser tab in the ordinary panel).
- Phases 6 and 7: running with the app closed, connected apps, voice, phone.
