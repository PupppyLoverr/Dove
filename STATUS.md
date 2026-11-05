# Keplorayx on zgx: where it stands

Branch `zgx/phase-a` in `C:\dev\keplorayx-zgx`, local only. Nothing is pushed, and `zgx/main` is untouched. The original Dove folder and `C:/dvw/target-ui-v4` were never written to.

This page says what works, what's partial and what's next. Each claim is backed by something you can run or a PNG in `apps/keplorayx/docs/design/shots/`.

## Run it

```bash
cargo build -p keplorayx -p dove-escrow
cargo run -p keplorayx                     # First Light on a first run, else the main window
cargo run -p keplorayx -- main             # the main window
cargo run -p keplorayx -- gallery          # Orbit's workbench
cargo run -p keplorayx -- onboarding       # setup on its own
cargo run -p keplorayx -- notch            # the notch on its own
```

- **Data:** `%LOCALAPPDATA%\keplorayx-zgx-dev`, unless `DOVE_HOME` says otherwise.
- **Builds:** `C:/dvw/target-kx-zgx`.
- **Without agents:** `KX_FIXTURE=<name>` runs a screen on stand-in data (`ui/fixtures/`): `home`, `thread`, `approval`, `review`, `files`, `browser`, `history`, `pulls`, `terminal`, `inbox`, `team`, `settings`, `constellation`, `first_run`, `notch`.
- **Off for tests:** `KX_NOTCH=0` (the notch), `KX_TRAY=0` (the tray and the keys from any app).

## The look: Zeron's, in Keplorayx's words

The interface is now built on **Zeron's design system, vendored whole** as `apps/keplorayx/ui/ds` (MIT, credited in `THIRD_PARTY_NOTICES.md`), with its strings reworded for Keplorayx (threads, not chats or sessions). Its tokens take their values from Orbit's two themes (`ui/kx/themes`: Zeron's charcoal and paper glass with Keplorayx's starlight accent and warm attention colour). Its icons draw Keplorayx's own glyph wherever one has the name. Type is Geist and Geist Mono, with Instrument Serif for the few large words.

| Part | What it is now |
|---|---|
| **Sidebar** | Zeron's `ds/sidebar`, worn as Dove's was: New thread as a lifted button with its keys, Search, Inbox, Constellation, Team; threads on two lines (agent mark, title, `project · branch`), the corner saying working, waiting on you, failed, done unseen, or how long ago. Pinned first. Its "…" menu groups by project, one line a thread, and the rest of Zeron's choices. Ctrl+B folds it. |
| **Composer** | Zeron's `ds/composer`: the draft over +, the microphone and Send; under it the access mode (with plan mode), the project (or the thread's branch) and the model. |
| **Model picker** | Zeron's, fed from the engine (`kx.models`): a tab for each installed agent, its models, an effort slider. Picking a model picks its agent. |
| **Approvals and questions** | Inside the composer, as Zeron asks them (Deny, Always allow, Allow; a question's options, or say something else). Y and N still answer from the keys. |
| **Thread** | Zeron's `ds/transcript`: your bubbles at the right, each reply's tools grouped between its words, the working line with its rolling word and clock, virtualized and following the end. |
| **Home** | A night sky under Zeron's dither (four skies, five effects; Settings › Appearance), the greeting in the serif, the composer over it with its tray. |
| **Ctrl+K** | Zeron's command palette over threads, actions, places and settings (Theme ›, Sky ›, toggles). |
| **Page and panel** | Two cards set into the window, as Zeron sets them and as Dove's page sat. |

## What's done

### The engine and windows (Phases A and C) ✅
- Dove's engine runs in process; transcripts stream from live sessions; automations run on the minute schedule.
- Remote-host launchers and `keplorayx node …` work as Dove's did.
- Inbox, Constellation, accent stars and First Light (overlay and six setup steps) as before.

### Panels (Phase D) ✅
One at a time beside any page, under a row of icon tabs; the panel's edge drags its width.
- **Changes** (Ctrl+J): the thread's diff against where it started.
- **Files** (Ctrl+P): the project's tree with git marks, find-a-file, and a viewer: code highlighted by Zeron's highlighter, pictures, Office documents' words (Word, PowerPoint, Excel), anything else opened in its own app.
- **Browser** (Ctrl+Shift+B): a real page (WebView2 on Windows) with Back, Forward, Reload and the address (words search); one page a project, kept with its history.
- **Terminal** (Ctrl+`): a shell in the project.
- **History**: the commit graph, refs and tags, find a commit; a click copies its id.
- **Pull requests**: the project's, through `gh` (it says so when `gh` is missing).

### Settings, Team, account (Phase E) ✅
- **Settings:** General (your name, setup again, keys from any app), Appearance (theme, glass, your star, the sky and its effect, motion), Agents (installed, default access), Notifications, Shortcuts, Account, Privacy and data (where it lives, what leaves), About.
- **Team** (Ctrl+Shift+T): the agents Keplorayx knows, installed or not; teammates (Dove's personas) with a name, role, preferred agent and brief; **Start a thread** runs one as that teammate.
- **Account:** the Woozlit sign-in, unchanged: the `dove-mesh-host` sidecar signs in through the browser. Sign-in only. Publishing threads to your phone stays off until the "bring or fresh" offer is ported.
- **Notifications:** the system's word when a thread needs you or finishes while you're away; the taskbar badge counts what waits.

### Beside the desktop (Phase F) ✅
- **The tray:** open, New thread, Inbox, Settings, Quit.
- **Keys from any app (Windows):** Ctrl+Alt+Space asks; Ctrl+Alt+A and Ctrl+Alt+D allow or deny what an agent waits on.
- **Voice:** the microphone types by Windows' voice typing (Win+H).
- **The notch** at the top of the screen, as before.

## Checks

- `zgx lint ui`: **0 errors, 0 warnings in 211 files** (the vendored design system included).
- `cargo test -p keplorayx`: **9 passed** (transcript fold, panel paths, Office text).
- Every fixture above and every window (gallery, onboarding, intro, notch) loads with no script error.
- Hardware numbers: `apps/keplorayx/docs/perf/README.md`.

## Not done yet (honestly)

- **Dove's MCP bridge (`--mcp-browser`):** not served, so agents start without Keplorayx's own tools (driving the panel's browser, the team tools). It's tied to Dove's GPUI browser and integration pages; it needs the Phase 1 extraction into `crates/dove-app`.
- **Remote control:** signing in works; publishing threads to the phone (the "bring or fresh" offer) isn't ported.
- **Clicked through by hand:** composer sends, the model picker's choice reaching a real agent, approvals from the composer, Files/Browser/History against a real project, the tray and keys. Built, loaded and screenshotted on fixtures; please try them live first.
- **Global keys on macOS and Linux, and the tray on Linux:** cfg-gated stubs. Nothing was built for macOS or Linux, as asked.
- **The gallery** still shows Orbit's own components, not the vendored ones.
