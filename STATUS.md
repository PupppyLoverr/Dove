# Keplorayx on zgx: where it stands

Branch `zgx/phase-a` in `C:\dev\keplorayx-zgx`, local only. Nothing is pushed, and `zgx/main` is untouched. The original Dove folder and `C:/dvw/target-ui-v4` were never written to.

This page says what works, what's partial and what's next. Each claim is backed by something you can run, a PNG in `apps/keplorayx/docs/design/shots/`, or a test.

## Run it

```bash
cargo build -p keplorayx -p dove-escrow
cargo run -p keplorayx                     # First Light on a first run, else the main window
cargo run -p keplorayx -- main             # the main window
cargo run -p keplorayx -- --background     # the tray, the notch and widgets; the window hidden
cargo run -p keplorayx -- launcher         # the launcher on its own, shown
cargo run -p keplorayx -- gallery          # Orbit's workbench
cargo run -p keplorayx -- onboarding       # setup on its own
cargo run -p keplorayx -- notch            # the notch on its own
```

- **Data:** `%LOCALAPPDATA%\keplorayx-zgx-dev`, unless `DOVE_HOME` says otherwise. Plugins in `<data>/plugins`, widgets in `<data>/widgets.json`, the calendar in `<data>/calendar.json`, agents' visuals in `<data>/visuals`.
- **Builds:** `C:/dvw/target-kx-zgx` (the sign-in helper: `C:/dvw/target-kx-zgx/mesh`).
- **Without agents:** `KX_FIXTURE=<name>` runs a screen on stand-in data (`ui/fixtures/`): `home`, `thread`, `visual`, `approval`, `review`, `files`, `browser`, `history`, `pulls`, `terminal`, `inbox`, `team`, `settings`, `constellation`, `first_run`, `notch`.
- **Off for tests:** `KX_NOTCH=0` (the notch), `KX_TRAY=0` (the tray and the keys from any app). `KX_MCP_ANNOUNCE=1` prints the agents' bridge port, so a script can play the agent.

## Beside the desktop

| Part | What it does |
|---|---|
| **The tray** | Closing the window hides it there: agents keep working, the notch stays. Its menu: Open, New thread, Inbox, Settings, Show the notch, Start with Windows, Quit. |
| **`--background`** | Starts with the window hidden (what Start with Windows runs, through a per-user Run entry). |
| **The notch** | An island hanging from the screen's top edge, shown only when the pointer reaches the edge over it (and slid away soon after); hidden, clicks pass through. It says who waits on you (Allow, Deny), else your agents at work, else the time, the day and Alt Space to ask. While something waits, a thin warm line stays at the edge. Opened, its tabs: Home, Agents, Music (Windows' media controls: cover, progress, play, skip, seek), **Tray** (files dropped on the island, kept until you take them away; Open, Show in folder), **Alerts** (the last 30 finishes, failures and asks, each opening its thread), Notes, Timer. On the same frosted glass as the window. |
| **The launcher (Alt+Space)** | Zeron's command palette in a window of its own over everything: apps (Start menu and packaged), open windows, files in your usual folders and projects, what you copied this run (memory only), threads, projects, quick links, snippets, safe system commands; sums and unit conversions as you type; web search; **Ask Keplorayx**, its answer streaming in place; **Now playing** (play, pause, next, previous); what you picked lately first again, kept across launches. Notes: `apps/keplorayx/docs/design/launcher-study.md`. |
| **Widgets** | Windows of glass on the desktop or floating: clock (digital, analog, world, small), calendar (agenda, month, small), agents at work, recent threads, quick ask, usage, CPU and memory (bars, small, graph), now playing (with the media keys), note, and an agent's own view. Drag them anywhere (kept); a secondary click changes the variant, floats it or removes it. Settings › Widgets adds them. |
| **Keys from any app (Windows)** | Alt+Space the launcher (a low-level keyboard hook: Windows keeps Alt+Space for window menus, so a registered hot key didn't fire); Ctrl+Alt+Space asks; Ctrl+Alt+A and Ctrl+Alt+D allow or deny. |

## Agents' tools: the bridge

`keplorayx mcp` (src/mcp.rs) is an MCP server every agent Keplorayx starts gets (Claude Code and Codex through their adapters, Gemini and OpenCode through ACP), relaying to the running app on a loopback port with a token.

- **Generative UI:** `show_visual` runs the agent's Luau script in Zeron's sandbox (`crates/visual`, vendored) before it's shown, and the thread draws it with the design system: stats, tables, charts, tabs, toggles, sliders, canvases. `show_image` shows a picture. **Not ported:** 3D scenes and shaders (Zeron renders them on the GPU); they say so in place.
- **Plugins:** `create_plugin`, `write_plugin_file`, `read_plugin_file`, `list_plugins`, `reload_plugins` (with the window's errors).
- **Widgets and calendar:** `set_widget`, `remove_widget`, `list_widgets`, `calendar_add`, `calendar_remove`, `calendar_list`.
- **Plugins' own tools:** every allowed plugin with an MCP program has its tools relayed, under its name.

## Plugins

On Zeron's model (`src/plugins.rs`): a folder with `kx-plugin.json` and `init.luau`, run by zgx in a sandbox of its own in the main window. **Settings › Plugins** is Zeron's page: search beside the title, Installed and Library tabs, each plugin a row with its tile and version; Install from the library, Allow (a dialog saying what it may do), a switch once allowed, Remove behind a confirm. An update asking for more asks again. Agents make plugins; you allow them. **Settings** as Zeron's: declared in the manifest (switch, choice, text, number), drawn under the plugin's name on that page, read by the plugin (`kx.setting`, `kx.on_setting`). Every plugin made gets `kx.luau`, the helpers (`place`, `toast`, `store`, `every`, `open`, `compose`, `threads`), after Zeron's SDK. Guide: `apps/keplorayx/docs/plugins.md`.

**Computer use** is in the catalog (`apps/keplorayx/plugins/computer`), not bundled as running: Install, then Allow. It runs Cua's driver (`cua mcp`) and gives whichever agent runs its 35 computer tools, with guidance to read the accessibility tree before any screenshot. Type `@computer` in the composer to ask for it.

## The look: Zeron's, in Keplorayx's words

Zeron's design system, vendored whole as `apps/keplorayx/ui/ds` (MIT, credited in `THIRD_PARTY_NOTICES.md`), with Orbit's two themes and Keplorayx's glyphs.

| Part | What it is now |
|---|---|
| **Sidebar** | Zeron's, calmer: New thread and Search on top; projects (each tile its own hue) with their threads on tree lines; Constellation, Team and Settings as quiet icons at its foot. A thread's menu: Pin (kept in the store), Open, Archive; double-click renames. |
| **Composer** | Zeron's, in three placements (centre, bottom, slim). Sending while the agent works **queues** the message in its dock (send now, edit, remove, reorder); it goes when the turn ends. `@` offers projects, agents and allowed plugins' mentions. |
| **Model picker** | Zeron's, fed from the engine: a tab for each installed agent, its models, an effort slider. |
| **Thread** | Zeron's transcript; agents' visuals drawn in place. Its head, as Zeron's, always shows the agent's mark and the title. |
| **Right panel** | Changes, Files, Browser, Terminal, History, Pull requests, on Zeron's tabs. Its home: this thread (project, branch, its changes), the project's other threads (Zeron's lanes), and what opens here, each with its key. |
| **Settings** | General (send with, compact, Stop agent with Escape, close the panel with its last tab, open links in the panel's browser or yours, hide emails, tray, notch, Start with Windows, keys), Appearance, Agents, Notifications, Shortcuts, **Plugins**, **Widgets**, Account, Privacy and data, About. |

## Fixed this round

- **Chat:** a follow-up's turn was missing from the window (a new session began with an empty transcript, and the old one put its own back); the open thread's record could keep the previous thread's id after a new thread started. Both found with the fake agent and fixed.
- **Notifications** never fired (they asked for a window property that doesn't exist); they do now.
- **Adding a project** refused folders whose top holds only folders.
- **Toasts** stack at the window's foot (they were laid out at the top, under the notch); repeats replace each other; errors clear after 7 seconds.

## Checks

- `zgx lint ui`: 0 errors, 0 warnings in 218 files. `cargo test -p keplorayx`: 15 passed.
- Every fixture, the launcher and the gallery load with no script error; the notch and First Light captured clean.
- Driven live on a scratch home: sending, follow-ups, the queue, the bridge's every tool (as an agent would), an allowed plugin drawing in the sidebar, widgets, closing to the tray, `--background`, the notch hidden until reached for, sign-in to the browser hand-off and Cancel.
- **Hardware numbers:** not re-run this round, as asked; the last are in `apps/keplorayx/docs/perf/README.md`.

## Not done yet (honestly)

- **Alt+Space** hasn't been tried with a real keypress: the hook ignores injected keys on purpose, so only you can. If an old Keplorayx is still running from before, quit it from its tray first: only one process can own the key.
- **Computer use on this computer** needs your one-time consent in Cua (`cua do-host-consent`, then `cua do switch host`): I didn't grant it for you. The plugin's tools reach agents; acting on the desktop waits on that.
- **3D scenes and shaders in visuals** (Zeron's GPU renderer, `crates/visual-gpu`).
- **Remote control:** signing in works; publishing threads to the phone isn't ported (left out, as agreed).
- **The launcher's** window management, emoji, file previews, images in clipboard history; plugins adding to the launcher.
- **Widgets:** the world clock's zones are fixed offsets (no daylight saving); now playing reads known players' window titles (Spotify, foobar2000, VLC, AIMP, MusicBee, iTunes), not Windows' media session.
- **A zgx crash** seen once: "stale scene node" in zgx's scene, while the window was still starting and a test changed its route. Not reproduced since; reported here so it isn't lost.
- **Dove's browser tools for agents** (driving the panel's browser) and its team tools aren't on the bridge yet; the bridge is Keplorayx's own, so the Phase 1 extraction into `crates/dove-app` is no longer in its way.
- **The gallery** still shows Orbit's own components, not the vendored ones.
- **Tested by driving the window and the bridge, not by hand:** sending, follow-ups, the queue, plugins, widgets, the launcher's search, the tray hiding. Approvals from the composer and a real agent calling the tools are still worth trying yourself.
- **Global keys on macOS and Linux, and the tray on Linux:** cfg-gated stubs. Nothing was built for macOS or Linux, as asked.
