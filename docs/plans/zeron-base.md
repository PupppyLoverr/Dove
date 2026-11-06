# Plan: Keplorayx on Zeron's app and engine

Goal: the whole Zeron app (its zgx interface and its engine) becomes Keplorayx's
base. The engine is changed to fit Keplorayx. Everything Keplorayx added is kept
and carried over, and the island and the launcher are redesigned after Apple's
Mac Dynamic Island and Tahoe Spotlight.

## What Zeron is

- **App** (`apps/zgx`): about 545 Luau screens plus a 52k-line Rust host. It
  talks to the engine over a local WebSocket RPC.
- **Engine**: `crates/engine` (146k lines), `harness` (78k), and `proto`,
  `rpc`, `client`, `sync`, `mcp` and smaller crates. It runs the agents and
  owns sessions, repos and worktrees, terminals and plugins. It handles
  sign-in through Zeron's servers (WorkOS).
- **Agents built in:** Claude Code, Codex, Cursor, OpenCode, Pi, Muse, and
  ACP agents (Gemini and others).
- **License:** MIT. It runs on Windows already.

## Ground rules (kept)

- Your Zeron folder is only read, never written. I copy from it as it is now,
  including your uncommitted edits. (Its source is already copied into
  `zr/` in our repo; nothing else has started.)
- The Dove folder and `C:/dvw/target-ui-v4` are never touched. Builds go into
  `C:/dvw/target-kx-zgx`, Windows debug only.
- **Sign-in stays Woozlit/Cosmos.** Zeron's account (WorkOS) is replaced in
  the engine. Zeron features that only work with Zeron's servers are turned
  off and hidden, not faked: sync across devices, Zeron Cloud sandboxes,
  members and billing, and the Zeron GitHub App.
- Local commits on `zgx/phase-a` only. No pushes. Credit Zeron in
  `THIRD_PARTY_NOTICES.md`.
- The current Keplorayx stays buildable as `keplorayx-classic` until the new
  one is proven, so you always have a working app.
- Before swapping the running app, I check that no agent is mid-run.

## Phases

### 1. Bring Zeron in
- Copy `apps/zgx` to `apps/keplorayx-next`, and copy the engine crates under
  `crates/zr-*`, renamed so they can sit next to ours.
- Point everything at the zgx we already vendor (our patched copy).
- Build on Windows. Fix whatever only built on macOS.

### 2. Make the engine Keplorayx's
- **Identity:** named Keplorayx, its own data folder
  (`%LOCALAPPDATA%\keplorayx`), its own local port, so it never meets a real
  Zeron install. App id `com.keplorayx.app`.
- **Sign-in:** Woozlit/Cosmos through our existing sign-in helper, in place of
  WorkOS. Turn off the parts that need Zeron's servers.
- **Agents:** keep Zeron's. Add the ones Keplorayx has and Zeron doesn't
  (Grok, Devin, Antigravity, Hermes, and Gemini if ACP doesn't already cover
  it) through ACP or a port from our `dove-core`.
- **Your data:** import your current Keplorayx threads, projects and settings
  into the new engine on first launch (Zeron has a local-import path). The old
  data is left untouched.

### 3. Rebrand the interface
- Our icon set everywhere, the Orbit night and dawn themes, the name, and our
  sidebar and composer tweaks.
- Zeron's screens are otherwise kept whole, as you asked.

### 4. Carry over everything we added
- **The island:** the window, the real glass and the chat with agents.
- **The Alt+Space launcher.**
- **Desktop widgets:** one surface on the desktop, with variants and editing.
- **Windows media controls:** now playing, with play, pause and skip.
- **Background running:** the tray, Start with Windows, and running in the
  background.
- **Keys from any app:** Alt+Space opens the launcher; Ctrl+Alt+A and
  Ctrl+Alt+D allow or deny.
- **Notifications** with the app identity.
- **Agents' tools:** show visuals and images, widgets, calendar, and making
  plugins. These are added to Zeron's MCP layer.
- **Computer use:** an installable plugin that adds `@computer`.
- **Settings rows:** Escape stops the agent, links open in the panel's
  browser, hide emails, and similar.
- **The composer's three placements** on a new thread: centre, bottom and
  slim (one line until you write more). Also its queue, `@` mentions for
  plugins and agents, and the effort setting following the model.
- **Our sidebar and settings changes:** grouped settings pages, jump keys,
  the archived shelf, the rail, and the Plugins and Widgets pages.

### 4a. Onboarding, sign-in and setup, as Zeron's
- **Zeron's whole first-run flow, kept:**
  - The intro played over the desktop: a scene decoding out of characters,
    the agents rising from the horizon, the words and the way in.
  - Then sign-in, the look (theme and accent), your username and your
    member card.
  - Then naming your organization and inviting to it, GitHub, and the
    agents (installing and signing in to each).
  - Then "You're all set".
  - Both of Zeron's paths: signed in, or without an account.
- **Restyled as Keplorayx:** our name, words, icons, Orbit colours and
  glass, with our own intro scene. It stays very close to Zeron's in shape
  and pace.
- **The new opening:** setup opens as a small glass card in the middle of
  the screen and expands outward into the full window. Each step then
  slides and morphs inside it, the way Zeron's steps do.
- **Sign-in is Woozlit**, in Zeron's sign-in screen and browser hand-off.
- **The member card:** Zeron's card (its scene, foil, sky, number, name,
  join date and badges) is dealt to you right after your username, with
  Share under it. Zeron makes the card on its server, so Keplorayx makes it
  locally from your Woozlit account, using Zeron's own card design and
  seeds. The card also lives in Settings › Account.

### 4b. Cloud, in Zeron's style
- Zeron's Cloud screens and flow are kept: a checkout option to run a
  session on its own sandbox, which sleeps when idle and shows its status
  in the sidebar.
- Zeron runs those sandboxes on its own servers, so Keplorayx backs the
  same screens with **cua sandboxes**: on this computer, or in your own
  cloud account (AWS, GCP, Modal) if you set one up. Same look and steps,
  our backend.

### 4c. Widgets, all glass
- Every widget becomes a glass card, using the island's own glass: what's
  behind it on the desktop, blurred, under a tint, sheen and rim.
- Each kind is improved: clock (analog, digital, world), calendar, agents,
  usage, ask, threads, system, music, note and custom. Each keeps its
  variants, and agents can still edit them.

### 4d. Research first
- Before designing the island and Spotlight, I study Apple's own pictures
  and videos: the touchscreen MacBook Pro's Dynamic Island (compact,
  minimal and expanded states, live activities, the Siri droplet) and
  Tahoe's Spotlight window (the rail, chips, actions, clipboard).
- I save notes and screenshots in `docs/design/` and design from them.

### 5. The island, as the Mac Dynamic Island
- Three states as on the Mac: compact (something at each end of the pill),
  minimal, and expanded. Spring morphs with a slight squish.
- Live activities: agents, music, a timer.
- Touch-friendly targets, tap to expand, and a press-and-hold menu.
- An assistant "droplet" that grows out of the island for Ask.
- Everything we already have: chat, Allow and Deny, the tray, alerts, and the
  waveform tinted from the cover.
- The same real glass, refined.

### 6. Spotlight, and better
- One glass window with a tint, sheen and rim.
- A category rail: Apps, Files, Actions, Clipboard, plus our Agents, with
  ⌘1–5 (Ctrl on Windows).
- Filter chips, Tab to search inside one app, and `/pdf`-style type filters.
- Actions that take fields (Tab between them), quick keys, ↑ for past
  searches, and Ctrl to show a result's path.
- Clipboard history with a retention setting (30 min, 8 h, 7 days, or off).
- A preview pane for the chosen result.
- Asking agents, with the answer streaming in place, and jumping to threads.
- Music, plugins and widgets in results.

### 7. Prove it, then switch
- Tests, clippy and lint.
- Drive every screen on a scratch home with a stand-in agent, and take
  screenshots of the island, the launcher and the main screens.
- Try Woozlit sign-in up to the browser hand-off. I won't type passwords.
- Import a copy of your data, then switch your running app (only if no agent
  is mid-run).
- Update `STATUS.md` and write an honest morning report: what's done, what's
  partial, and the screenshots.

## Honest risks

- **Size:** this is far bigger than anything we've done so far. The first
  Windows build of Zeron's engine alone may take 15 minutes or more. I'll work
  in the order above so a working new app exists as early as possible. If
  something isn't finished by morning, the report will say exactly what.
- **Servers:** Zeron's server-backed features that have no local stand-in
  (sync across devices, billing and members on Zeron's servers, the Zeron
  GitHub App) won't work with Woozlit; they are hidden. Cloud and the
  member card get local stand-ins (cua sandboxes, a card made on this
  computer).
- **Hardware:** no hardware tests, as before. Real Alt+Space presses, music
  playing and touch can only be checked by you.
