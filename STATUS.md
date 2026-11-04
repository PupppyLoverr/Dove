# Keplorayx on zgx: where it stands

Branch `zgx/phase-a` in `C:\dev\keplorayx-zgx`, local only. Nothing is pushed, and `zgx/main` is untouched. The original Dove folder and `C:/dvw/target-ui-v4` were never written to.

This page says what works, what's partial and what's next. Each claim is backed by something you can run or a PNG you can look at.

## Run it

```bash
cargo build -p keplorayx -p dove-escrow
cargo run -p keplorayx                     # First Light on a first run, else the main window
cargo run -p keplorayx -- main             # the main window
cargo run -p keplorayx -- gallery          # Orbit's workbench: every token and component
cargo run -p keplorayx -- onboarding       # setup on its own
cargo run -p keplorayx -- intro            # First Light's arrival over the desktop
```

- **Data:** `%LOCALAPPDATA%\keplorayx-zgx-dev`, unless `DOVE_HOME` says otherwise.
- **Builds:** `C:/dvw/target-kx-zgx`.
- **Without agents:** `KX_FIXTURE=<name>` runs any screen on stand-in data, from `ui/fixtures/`: `first_run`, `home`, `thread`, `approval`, `review`, `inbox`, `settings`, `constellation`.
- **One setup step:** `KX_STEP=<step>` opens setup on that step.

## What's done

### Phase A: proof run ✅
- **Workspace:** GPUI is removed, zgx is pinned at `f01b47a0`, and builds go to their own folder.
- **Host:** Dove's engine runs in process. One Inset-titlebar window sits over Mica, and transcripts stream from live sessions.
- **Automations:** they run on Dove's minute schedule, ported from the GPUI shell.
- **Tests:**
  - `cargo test -p keplorayx`: the transcript fold, including the new approval test.
  - `cargo test --workspace --exclude keplorayx`: **898 passed, 0 failed**.

### Phase B: Orbit, the design system ✅ (first pass)
- **Themes:** night and dawn (`ui/kx/themes/*.json`), restyling in place as they're saved.
- **Tokens:** Keplorayx's own (`kx.*`), declared in standard terms so any theme styles them.
- **Type:** Inter, Inter Display and JetBrains Mono, bundled under `fonts/` (OFL).
- **Motion:** one reduce-motion check, Orbit's settle-and-overshoot curve, and one shared breath clock that runs only while something is live.
- **Shape:** concentric corners (`kx/shape`), and the lip of light along every lifted surface.
- **Icons:** Dove's library refined into one set with an optical stroke, plus 17 new glyphs drawn for Keplorayx. Made by `tools/icons.py`; see the gallery's Icons page.
- **Components:** Button, Field, Switch, Segmented, Pill, Row, AgentMark, Kbd, Empty, Card, Float, Menu, Popover, Dialog, Tooltip, toasts.
- **Gallery:** `-- gallery`, both themes. PNGs in `apps/keplorayx/docs/design/shots/gallery-*`.
- **Lint:** `zgx lint ui` reports 0 errors and 0 warnings. The design lint runs on every entry.

### Phase C: daily use ✅ (core)
- **Sidebar:** the way in, then threads by project led by each agent's star, then Settings and "Local only" at the foot.
- **Home:** a greeting, the **Ray bar** (ask; `>` commands; `@` projects and agents; `/` agent commands), the project and agent chips, three first prompts, and what's under way. A first run gets one way in (choose or drop a folder).
- **Threads as beats:** prompt, thinking, tools on a spine with +/− file chips, and the reply at a ~68-character measure as the draft streams.
- **Approvals:** a warm card above the composer (Y allows, N denies).
- **Inbox:** what needs you, then what finished (J/K to move).
- **Settings:** Appearance, Agents, Privacy and account, Shortcuts, About.

### Phase D: panels (started)
- **Changes panel** (Ctrl+J): the open thread's diff against where it started, file by file, with hunks in mono.

### Phase E: the rest (started)
- **First Light** (`intro.luau`, `onboarding.luau`, `src/intro.rs`):
  - the overlay's arrival, then the hand-off to setup (prepare, reveal, arrived, done);
  - six steps: welcome (name, light, your star), your agents (ready, or install with a copyable command), your first project, your team, the notch and shortcuts, privacy;
  - it ends in the main window, and Settings opens setup again.
- **Constellation** (Ctrl+− or Ctrl+Shift+O): every thread as a body in orbit around its project.
- **Accent stars:** pick yours at setup or in Settings.

## What's not done yet (honestly)

- **D:**
  - not started: the terminal and flock, the WebView2 browser, files, PR view, history graph, office previews, Peek;
  - the Review deck is just the Changes panel so far.
- **E:**
  - not started: the ~10k lines of Dove's settings pages, Team and orchestra, voice, Live visuals, wallpaper, computer-use review, notifications;
  - not wired: the Woozlit sign-in (designed and kept, but the button is disabled until the sidecar bridge moves into `crates/dove-app`).
- **F:** the notch and island are not started. Setup has a preview of it.
- **Phase 1 extraction** (`crates/dove-app`): not done. Until then, Dove's MCP bridge (`--mcp-browser`) isn't served by `keplorayx`, so agents start without the browser tools.
- **Not built tonight:** macOS and Linux, as you asked. The code keeps them cfg-gated: Windows-only pieces sit behind `cfg(windows)`, and the Cargo features for macOS and Linux are in place.
- **`zgx check ui`** needs `luau-lsp`, which isn't installed. I didn't download it unasked. `zgx lint ui` passes, and every screen was loaded and shot.

## Performance

(Filled in below once both runs finished.)

## Where things are

- `apps/keplorayx/src/`: the host.
  - `main.rs`: modes and windows.
  - `host.rs`: the engine for the scripts.
  - `transcript.rs`: the fold.
  - `intro.rs`: First Light's hand-off.
- `apps/keplorayx/ui/`: the interface.
  - `kx/`: Orbit.
  - `screens/`: the screens.
  - `fixtures/`: stand-in data.
  - `app.luau`, `gallery.luau`, `onboarding.luau`, `intro.luau`.
- `apps/keplorayx/docs/design/`: the two studies and `shots/` (the PNGs).
- `apps/keplorayx/zgx-asks/`: zgx gaps found, each with a repro and its workaround.
- `apps/keplorayx/tools/`:
  - `shot.ps1`, `shots.ps1`: design PNGs;
  - `smoke.ps1`: launch and read errors;
  - `icons.py`: the icon library.
- `scripts/perf/perf.ps1 -Kind kx`: the RAM/CPU harness, which now measures keplorayx too.
