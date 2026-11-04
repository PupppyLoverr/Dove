# Keplorayx on zgx: a rewrite with a new look

## ▶ Handoff status (2026-11-04): read this first in a new window

**Done in `C:\dev\keplorayx-zgx`, branch `zgx/main`. Nothing is committed yet. The original Dove folder is untouched.**
- **Clone:** made from the Dove worktree's `main` (aa48d96).
  - `origin` = `github.com/PupppyLoverr/Dove`; `local-dove` = the original worktree.
- **Build folder:** `.cargo/config.toml` sets `target-dir = "C:/dvw/target-kx-zgx"`.
- **Root `Cargo.toml` rewritten:**
  - members are `crates/*` and `apps/keplorayx`; `apps/dove` is excluded and kept as the reference
  - GPUI dependencies and profiles removed
  - zgx/zgx-desktop/zgx-script/zgx-run pinned at `f01b47a00a6f5e9386b5fd7fb35dcdbef4991e93`
  - zgx's `[patch.crates-io]` block copied over
  - `rust-version` is 1.95; the installed toolchain is rustc 1.97.1
- **`vendor/gpui-base`:** removed with `git rm`.
- **`Cargo.lock`:** regenerated from scratch with `cargo generate-lockfile`, after `objc2`/`zerovec` clashed with zgx's exact pins. The old lock is saved in the old session's scratchpad as `Cargo.lock.dove`.
- **`apps/keplorayx/` created:**
  - `Cargo.toml`: bin `keplorayx`; zgx features `terminal`, `webkitgtk`, `webview2` on Windows, `ghostty` on macOS; `inspect`, `i18n`, `fetch`; `windows-sys` for the registry; `serde_json` as a dev dependency.
  - `src/main.rs`:
    - sets `HOME` from `USERPROFILE`, `ZGX_CHECK=0`, and `DOVE_HOME` to `%LOCALAPPDATA%\keplorayx-zgx-dev`
    - `Engine::new` runs in process, with `rescan_harnesses` on a thread
    - one window: `Titlebar::Inset` with `Glass` `UnderWindowBackground`
    - modes `main`, `gallery` and `--inspect`; scripts come from `ui/` (or `KX_UI`)
  - `src/host.rs`:
    - shared state `kx.threads`, `kx.projects`, `kx.thread`, `kx.entries`, `kx.draft`
    - actions `kx.open`, `kx.new`, `kx.interrupt`; async `kx.send`
    - live sessions are drained with `recv_async`, and changes are published per change
    - spawning goes through `zgx_desktop::TaskSpawner`
  - `src/transcript.rs`: the pure fold from `AgentEvent` to entries plus a draft, with 4 unit tests.
  - `src/transparency.rs`: ported from Zeron, with credit.
  - `ui/.luaurc`, `ui/app.luau` (sidebar, measured transcript list with `follow`, streaming draft row, composer), `ui/gallery.luau` (placeholder), `ui/kx/store.luau`.
  - `README.md`.
- **`THIRD_PARTY_NOTICES.md`:** adds a zgx (MIT) section and the Zeron-zgx port credits.

**Next steps, in order:**
0. **Before changing auth/account/sync code:** complete the read-only Zeron + Dove audit in “Additional required audit: Keplorayx identity, sign-in, sync and local-only mode”. Write the report and recommendations first; do not implement cloud/account changes without approval.
1. `cd C:\dev\keplorayx-zgx && cargo check -p keplorayx`. The first full compile of zgx was interrupted, so this is the first real compile of the new code. Expect API fixes in `host.rs` and `main.rs`:
   - `Value::from` for `bool` and `f64`
   - `Shared::with_untracked`
   - the shape of `async_action`'s return
   - whether `zgx_script::inspect::serve` returns a server to keep
   - `cx.inspector`
   - `TaskStatus::id`
   - `HarnessKind::label`

   Check against the zgx source in `~/.cargo/git/checkouts/zgx-*`, or Zeron's `apps/zgx/src/main.rs:791-925` (`serve_with`).
2. `cargo test -p keplorayx`: the transcript fold tests.
3. `zgx lint ui` and `zgx check ui`. Install with `cargo install --git https://github.com/zeronsh/zgx zgx-script --bin zgx --features inspect`.
4. Seed a fake-agent home: `python scripts/perf/seed_home.py`, after building `scripts/perf/fake-agent`. Then run `DOVE_HOME=<seeded> cargo run -p keplorayx` and check that a thread streams.
5. **Phase A gate:** port `scripts/perf/perf.ps1` to the `keplorayx` binary and compare with the GPUI build.
6. Commit Phase A on `zgx/main`. Then Phase 1 (`crates/dove-app` extraction) and Phase B (design, see below).
7. **Before any design work:** do the Zeron study in "★ The bar" below, and write `docs/design/zeron-study.md`.

**Design, viewable without Paper.** Paper's MCP isn't connected here, and HTML mockups are against your rule. So the Orbit design is made **in the app itself**:
- `ui/gallery.luau` holds every token and component, dark and light side by side (`cargo run -p keplorayx -- gallery`).
- Each screen gets a design fixture (`ui/fixtures/*.luau`) you can open with `-- fixture=...`. It reloads as it's edited, so you watch it change.
- After each design pass, offscreen screenshots are saved as PNGs to `apps/keplorayx/docs/design/` for review: `audit_shots`, following Zeron's `src/audit_shots.rs`.
- When Paper is connected again, the token sheet can be mirrored there.

## ★ The bar: the calmest, most polished interface and onboarding anywhere

You want Keplorayx on zgx to be the most calm, polished and beautiful interface and onboarding experience ever, at the level of `C:\dev\zeron`'s zgx app. Treat that as the **quality bar** for every phase, not as something to finish later.

**First, study Zeron's zgx app closely (read-only) before designing anything.** Learn *how* it reaches its polish, then do it Keplorayx's own way. Don't copy its look, art, palette, fonts or copy.
- `C:\dev\zeron\apps\zgx\README.md`, especially "The design system's rules", "Patterns" and "The intro's hand-off"
- the onboarding:
  - `ui/intro.luau`
  - `ui/screens/onboarding/` (one module per step, `state.luau`, `actions.luau`)
  - `ui/ds/onboarding_frame.luau`, `ui/ds/card_wall.luau`
  - `ui/ds/decode.luau` (a layer-shader transition)
  - `src/main.rs` `intro()` (the hidden-window hand-off, `prepare`, `reveal`, `done`)
- the motion and the feel:
  - `ui/ds/motion.luau` (one reduce-motion check)
  - `ui/ds/material.luau` and `glass.luau`
  - `ui/ds/tokens.luau`
  - concentric corners (`control.nest`/`contain`)
  - one wash for hover, press and selection
- how it keeps data changing in place: `ui/tests/*_in_place.test.luau` and `tests/reload.rs` (a hot reload changes nothing)
- the workbench: `ui/gallery.luau`. Run `cargo run --release -- gallery` in `C:\dev\zeron\apps\zgx` to see how finished it feels.

Write down what you learn in `apps/keplorayx/docs/design/zeron-study.md`: the techniques to adopt (as mechanisms), and what Keplorayx will do differently.

**What "calm" means in Keplorayx (applies to every screen):**
- **Quiet by default:** no badges, spinners or colour unless something needs you. The one warm attention colour is spent only on that.
- **One motion language:** every movement uses Orbit's eased overshoot on one shared clock. Nothing pops in. Lists slide, panels settle, text fades in by line as it streams, and nothing blinks while idle (idle frames = 0).
- **Type does the work:** generous line height, a measured reading width (~68ch) for transcripts, and hierarchy from weight and space rather than boxes.
- **Depth from light, not lines:** glass, soft top-edge light and layered surfaces. Hairlines only where they help reading.
- **Every state designed:** empty, loading, error, offline, very long, first run and a reduced-motion version, each shown in the gallery and as a PNG.
- **Respect the system:** reduced motion and transparency, high contrast, text size and the accent colour all followed. Fully keyboard- and screen-reader-able (AccessKit).
- **Micro-polish checklist** for every component: optical alignment of icons and text, concentric corners, even container insets, hover/press/focus/disabled states, and copy reviewed for warmth and brevity.

**Onboarding, "First Light":** a signature first run in Keplorayx's own voice.
1. **Arrival.** A transparent, frameless overlay over the desktop (zgx `covers_display`, always on top, clicks pass through until asked for). A dark sky fades in, and a single point of light, Keplorayx's mark, draws its orbit as the wordmark resolves. Capped at 60 fps, and skippable with any key.
2. **The hand-off.** The setup window opens hidden underneath (the same prepare/reveal technique as Zeron's `intro()`, written fresh). The orbit tightens into the window's rounded rect, and the window clears to its glass on the same frame. There are no visible seams.
3. **Steps.** One calm card each, with progress shown as small planets along an arc:
   - **Welcome:** your name, light or dark, and an accent colour picked from a few "stars".
   - **Your agents:** detected harnesses each appear as an orbiting body. Sign-ins are one click. Missing ones show "Install" with a copyable command.
   - **Your first project:** pick a folder (or drop one in). Keplorayx reads it on a thread, and the repo's name and language settle into the card.
   - **Your team** (optional): add your first teammate persona, with a preview of how they'll introduce themselves.
   - **The notch and shortcuts:** a live preview of the island and the global hotkey, which you can try right there.
   - **Privacy:** local-only by default, in plain words; clearly distinguish local storage from traffic sent to a user-selected model/provider or remote integration. Account sign-in and Keplorayx cloud sync are optional and never silently enabled.
4. **First thread.** Setup ends straight into Home with the Ray bar focused and three suggested first prompts for the project you chose. The first reply streams in as a demonstration of the calm transcript.
5. **Rules:**
   - Every step can be resumed (`KX_STEP=<step>`, as Zeron's `ZERON_ZGX_STEP`), skipped and reached again from Settings.
   - Each step has a headless fixture and a PNG.
   - Copy is short, warm and in plain words.

**Added to the phases:**
- **Phase B** also delivers the onboarding (intro overlay, hand-off and steps) as gallery pages and fixtures, with a "Zeron study" note and a review of PNGs and a short screen recording.
- **Phase C** gate: "a first-time user goes from launch to a streaming first thread with no confusion": run it with a scratch `DOVE_HOME`, and record it.


## ★ Additional required audit: Keplorayx identity, sign-in, sync and local-only mode

**Do this as a read-only investigation before implementing or changing authentication, account, sync, or privacy behavior.** Use `C:\dev\zeron` as a reference for implementation mechanisms and UX patterns, not as a product specification. Keep the original Dove source of truth read-only, and do not copy Zeron's branding, wording, visual design, endpoints, secrets, or account assumptions.

### Questions to answer

Inspect the relevant code in both `C:\dev\zeron` and the original Dove repository, then write `apps/keplorayx/docs/design/auth-sync-local-mode-study.md` with file paths and evidence for each finding:

1. **Authorization and sign-in**
   - How does Zeron implement authorization, sign-in/sign-out, session restoration, credential storage, token refresh/expiry, and error/recovery states?
   - Which parts are UI, host logic, provider integration, or shared infrastructure?
   - What is reusable as a general mechanism, and what is specific to Zeron's product or services?
   - Map Dove/Keplorayx's existing auth flows, especially `agent_auth.rs`, `mcp_oauth.rs`, harness sign-in, and integration-specific credentials. Preserve supported providers and existing behavior unless a deliberate change is approved.
2. **Sync and account data**
   - Does Zeron implement cloud sync, account-backed storage, or only local persistence? Identify the actual data flows rather than inferring from UI.
   - Determine which Keplorayx data could be local, synced, or explicitly excluded: settings, projects, threads, memories, credentials, team/persona data, and provider connections.
   - Identify conflict handling, offline behavior, migrations, deletion, and what happens when a user signs out.
3. **Local-only mode**
   - Define a real **Local-only** mode: local data stays on the device, no Keplorayx account is required, and Keplorayx does not upload app data or call Keplorayx-operated cloud services for that data.
   - Distinguish this from third-party model/provider traffic. If a user configures a hosted model, MCP server, browser page, or other remote integration, explain what data that service may receive and provide clear controls/disclosures.
   - Make Local-only the default unless existing product requirements explicitly establish otherwise. Any Keplorayx cloud sync must be a separate, explicit opt-in, with a clear explanation of what will sync and a way to disable it.
   - Verify behavior in code and with network-level tests. Do not treat a label or UI toggle as proof that a mode is private.
4. **Product UX**
   - Recommend a calm, understandable setup flow for local use, optional account sign-in, and optional sync. Avoid blocking local use behind account creation.
   - Specify signed-out, signed-in, sync-off, syncing, offline, auth-expired, sync-conflict, and sign-out states.
   - Explain whether sign-in is per provider/harness or for a Keplorayx account; do not conflate them.
5. **Security and data lifecycle**
   - Identify where secrets are stored and whether OS-protected credential storage is available on Windows and macOS.
   - Check logs, crash reports, diagnostics, backups, caches, exports, and deletion paths for accidental credential or private-data exposure.
   - Describe sign-out, disconnect-provider, clear-local-data, account deletion, and sync-disable semantics. Do not delete or migrate real user data during the audit.

### Required deliverable and guardrails

- Produce a concise architecture/data-flow diagram and a table with columns: **Capability / Zeron implementation / Dove implementation / Keplorayx recommendation / Evidence / Open questions**.
- Separate verified facts from assumptions and unanswered questions. Cite repository paths and relevant functions/types; do not claim a capability exists until code proves it.
- Recommend the smallest safe plan before making code changes. Ask for approval before introducing a new account backend, remote service, telemetry, cloud sync, or breaking auth behavior.
- Add tests for the selected design later: local-only network-denial tests, provider-auth regression tests, sync opt-in tests, and data lifecycle tests. Keep secrets out of fixtures and logs.
- Do not copy Zeron's account or sync implementation wholesale. Reuse only compatible, properly licensed mechanisms and retain required third-party notices.

## Context

Keplorayx (the Dove repo) is a Rust desktop app built on GPUI, through the zui fork and gpui-base. Its UI is the `apps/dove` crate: 114k lines in 139 files. You want it rewritten on **zgx** the way Zeron has been rewritten. That rewrite is `C:\dev\zeron\apps\zgx`, and its Windows support is your own work on `windows/zgx-parity`. You also want a **new UI/UX identity**: port mechanisms from Zeron where they help, but don't copy its look.

**Hard constraint:** the current Dove folder, its version and its builds (`C:/dvw/target-ui-v4`) must not be touched. Everything happens in a separate clone.

**Your decisions:**
- **Where:** a clone on a new branch.
- **Interface language:** Luau screens with a Rust host.
- **Engine:** in the same process.
- **Look:** a bold new identity, designed in Paper first.

**What makes this feasible:**
- Every crate under `crates/` is already free of GPUI. The UI holds an `Arc<Engine>`, drains `LiveSession.events` (a `flume::Receiver<AgentEvent>`) and calls `send(Command)` (`crates/dove-core/src/lib.rs:255`).
- zgx has what Dove needs on Windows:
  - Mica/Acrylic glass
  - the `Titlebar::Inset` caption buttons
  - WebView2 hosted in DirectComposition
  - a ConPTY terminal
  - Text Services Framework input
  - AccessKit exposed to UI Automation
  - always-on-top, click-through overlay windows
  - `gpu_view` on wgpu 29 (the same version as `zeron-visual-gpu`)

## 0. Workspace setup (no effect on current Dove)

- **Clone:** `git clone C:\Users\ravth\.zeron-zgx\worktrees\dove\quiet-lynx C:\dev\keplorayx-zgx`, then `git switch -c zgx/main` from current `main`.
  - Keep the original repo's remote as `origin` so the crates can still sync with Dove later.
  - Never push `zgx/main` to `main`.
- **Separate build output:** set `CARGO_TARGET_DIR=C:/dvw/target-kx-zgx` in the clone's `.cargo/config.toml`, so it never touches `target-ui-v4`.
- **Separate data:** dev builds default to `DOVE_HOME=%LOCALAPPDATA%\keplorayx-zgx-dev`, so agents, the store and settings never collide with the running Dove.
  - `dove-store`'s format stays identical, so a real install can later move onto it.
- **Separate binary:** call it `keplorayx` (not `dove.exe`), so locked-exe and kill scripts aimed at Dove never hit it.
  - Internal identifiers stay `dove-*`, per the rename rule.
- **Remove GPUI from the clone:**
  - exclude `apps/dove` from the workspace now (kept so files can be `git mv`'d into `crates/dove-app` with history); delete it after Phase 1
  - delete `vendor/gpui-base`
  - remove the `gpui`, `gpui_platform` and `gpui-base` workspace dependencies and the gpui profile overrides
  - the root workspace can then carry zgx's `[patch.crates-io]` (winit, wgpu-hal, accesskit) safely
- **Pin zgx:** use git `zeronsh/zgx` at the revision your branch uses (`f01b47a0`). Copy its patch block from `C:\dev\zeron\apps\zgx\Cargo.toml`.
  - Features: `zgx-desktop` with `terminal` and `webview2` (Windows), `webkitgtk`, `ghostty` (macOS); `zgx-script` with `inspect` and `i18n`; `zgx-run` with `fetch`.
  - Vendor `patches/zgx-terminal` (the box-drawing fix) as Zeron does.
- **New app crate:** `apps/keplorayx/`, laid out as follows.

```
apps/keplorayx/
  src/            Rust host: windows, engine bridge, Win32, notch, tray, hotkeys
    main.rs       window setup, the modes (main, gallery, island, mcp-browser)
    host/         engine → shared state, actions, async actions
    win/          ported Win32: transparency watch, notch input, SMTC media, hotkeys
    terminal.rs   terminal bridge to dove-term
    browser.rs    WebView2 control and the agent page API
  ui/             Luau, hot-reloaded
    .luaurc       strict; aliases @kx (design system), @screens
    app.luau  gallery.luau  island.luau
    kx/           Keplorayx's design system: tokens, themes/, components
    screens/      shell, home, thread, panels/, settings/, team, voice, notch
    tests/        *.test.luau, *_in_place tests
    fixtures/     headless stand-ins for every store
  docs/  zgx-asks/  (one folder for each framework gap, with a repro)
```

## 1. Shared Rust work before the UI

- Move the ~15.7k GPUI-free lines and the logic that still lives in the UI into a new `crates/dove-app` that the host links:
  - `orchestra`/`orchestrate`/`team_tools` execution
  - `remote.rs`
  - `browser/control.rs` and `agent.rs`
  - office parsing
  - `supermemory.rs`, `memory.rs`
  - `mcp_browser.rs`, `mcp_oauth.rs`, `integrations_catalog.rs`
  - `computer_plugin.rs`/`computer_host.rs`, `updates.rs`, `ego.rs`, `agent_auth.rs`
- Replace GPUI `Global`s (`ComputerUse`, `Memory`, `PrCache`, `RemoteGlobal`, `Host`…) with plain services owned by the host.
- **Engine bridge (`src/host/`).** For each live session, a `zgx::task::latest` task drains `events` and publishes **per entry**: `set_key` on the one message that changed, never the whole list again.
  - A streaming reply goes into a small separate draft state.
  - Pattern to port: `C:\dev\zeron\apps\zgx\src\chat.rs` (header, `publish_messages` around :1169) and `src\transparency.rs`.
  - Luau reaches the host through `ui.shared(...)` and `ui.call(...)`, wrapped by one store module per screen. Each store has a fixture for headless runs (Zeron's `settings_stub.luau` pattern).
- **Windows code to port from your branch:**
  - `transparency.rs`: the `RegNotifyChangeKeyValue` watcher on `EnableTransparency`
  - the title-bar / caption-button integration
  - the terminal bridge's Windows changes (raw VT mode, UTF-8, command-line quoting, typed keys sent as one write)
  - the fallback from `HOME` to `USERPROFILE`
  - Windows wording
- **Win32 code kept from Dove as it is:**
  - `notch/win.rs` (raw input, `RegisterHotKey`, clipboard listener)
  - `notch/media.rs` (SMTC, Core Audio meters)
  - `hotkey.rs`
  - `tray-icon` in `island/`
  - the named-mutex single instance
  - `PrintWindow` capture (`look.rs`)
  - `device.rs` DWM calls
- **Terminal:** keep `dove-term` (alacritty 0.26 + portable-pty) as the source of truth, because flock and agents read its grid. Draw it with zgx's `ui.terminal` through a bridge, as Zeron does.
- **Browser:** zgx's WebView2 `ui.webview` replaces zui's scene-overlay layer. Keep `page_api.js` and the control and agent logic.

## 2. Look and feel: "Keplorayx Orbit" (designed in the live gallery, reviewed as PNGs; see Handoff)

**Ground rules.** Nothing visual comes from Zeron:
- no `zeron.*` tokens
- no Geist or Instrument Serif
- no copied component recipes, palettes or art

What may be ported is *mechanism*, with credit in `THIRD_PARTY_NOTICES.md`, since Zeron's code is MIT:
- the host bridge
- virtualized and keyed lists
- the transparency watcher
- the terminal bridge
- the syntax lexer
- concentric-corner maths
- the test approach

**Concept.** Kepler's laws: each agent is a body in orbit around your work. Calm when idle, drawn closer when it needs you. It should feel like an instrument, not a chat app.

**Identity**
- **Type:** a sharp grotesk for the UI, a characterful mono for agent output and code, and a numeric face for timers and costs. Chosen in Paper and bundled under `fonts/`.
- **Colour:** a deep "night-sky" neutral scale with one luminous accent per agent (each agent's colour follows it everywhere: sidebar, notch, diffs), and a warm "attention" colour reserved only for things that need you.
- **Shape:** a "lens" radius family with concentric corners derived from tokens, and hairline borders that catch light at the top edge (like the notch's lip).
- **Motion:** orbital easing (overshoot settling into an ellipse) for panels; a single shared "breath" clock for every live indicator; reduced-motion respected.

**Signature experiences**
1. **The Ray bar.** One bar serves as the composer, Ctrl+K, search and slash commands. On Home it sits alone and centred (keeping the bar-only Home and 5 bar buttons). In a thread it becomes the pill composer. Typing `>` runs commands, `@` mentions files, apps, windows and teammates, and `/` gives the harness's commands. Results arrive as a live list under the pill.
2. **Constellation view.** Zoom out (pinch or Ctrl+-) from any thread to see every running agent as a node on a calm canvas: grouped by project, lines for teammates and orchestration, the size of each node showing how active it is, and a pulse when it needs you. Click to dive back in. This replaces flock's tiling as the overview; flock tiles remain for the terminal UIs.
3. **The Attention inbox.** One queue of approvals, questions, finished runs and failures. Triage by keyboard: J/K to move, A to approve, R to reply, E to archive. The notch mirrors it, and the sidebar shows a count on the agent's own colour.
4. **The transcript as beats.** Each agent turn becomes a "beat": a header line (what it set out to do), a compact spine of tool calls (expand for detail), inline file-change chips with +/− counts, and the reply in streaming markdown. Long runs collapse into a rolling summary; a scrubber on the right edge jumps through beats.
5. **Review deck.** Changes show as a stack of hunk cards with a narrated walkthrough (`generate_walkthrough` already exists). Approve, revert or comment on each hunk; the deck follows the agent's narration.
6. **Peek.** Hover any file, diff, PR or URL chip and a glass popover shows a live preview: the file viewer, the diff, or the page through WebView2.
7. **Live visuals.** Agent visuals (Luau visuals from `zeron-visual`) render as real zgx nodes in a sandbox instead of rasterized images, so they are sharp, can be interacted with and follow the theme.
8. **The notch, upgraded.** The same island, now a zgx overlay: always on top, transparent, clicks pass through, with the lip of light tied to the shared attention state, and quick replies typed straight into it.

**Kept from your earlier decisions:** the glass sidebar, the pill composer in threads, the centred Home card (redrawn in the new language), and Team/teammates naming (never Crew/Captain).

**Design deliverables (gallery pages + design fixtures + PNG shots in `apps/keplorayx/docs/design/`), before building screens:**
- a token sheet: type, colour (dark first, then light), radii, spacing, motion curves
- Home, a thread with beats, the Ray bar's states, Constellation, the Inbox, the Review deck, Settings, and the notch states

They are then turned into `ui/kx/themes/*.json` and `ui/kx/tokens.luau`.

## 3. Build phases (each ends runnable, with a gate)

| Phase | Builds | Gate |
|---|---|---|
| **A. Proof run** | Clone, set up the workspace, a window with the Inset title bar and Mica, the engine in process, a session list and one streaming transcript from the fake agent (`scripts/perf/`) | Idle frames = 0; keypress latency p95 ≤ the GPUI build; binary within the 115 MB gate; streaming CPU ≤ GPUI |
| **B. Design system** | The Paper sheet becomes tokens and themes; `kx/` components (text, button, surface, pill, list row, popover, menu, toast, tooltip, dialog, tabs, icon with Solar SVGs, glass); `gallery.luau` in both themes; 30 themes and the VS Code theme import moved to token JSON | Gallery passes `zgx lint` and `zgx check`; screenshots approved |
| **C. Daily use** | Shell, sidebar, Home with the Ray bar, thread with beats (tool calls, markdown, maths via `zatex`, Mermaid), composer, picker and mentions, attachments, split threads, approvals, the settings frame | You can work in it for a full day; `_in_place` tests green |
| **D. Panels** | Changes, diff list, history graph, files, PR view and pull requests, terminal and flock, browser (WebView2), office previews, the Review deck, Peek | Matches Dove's panels feature for feature |
| **E. The rest** | All ~10k lines of settings pages, Team, orchestra and race, voice call and dictation, Live visuals, wallpaper and Home media, the intro, computer-use review, notifications, Constellation, the Inbox, and the approved auth/sync/local-mode design | Feature checklist built from Dove's module map all ticked; local-only and provider-auth tests pass; any Keplorayx cloud sync remains explicit opt-in |
| **F. Notch and island** | `--island` process on zgx (overlay window), tray, hotkeys, media, quick reply | Notch contact sheet (`scripts/notch_sheet.ps1` rewritten) approved |
| **G. Release** | Bundling `ui/` into the release build (`zgx_script::build::bundle`), Windows packaging and signing, a move-over path for `DOVE_HOME`, updater | Clean install on a fresh Windows user; data from the current Dove carries over |

**Gaps to work around and send upstream (`zgx-asks/`):**
- the zui-only effects (`with_edge_fade`, `paint_glyph_transformed` for `roll_text`, masked fitted images): rebuild them with zgx layer shaders or masks
- glass behind pop-ups on Windows (draw it solid until fixed)
- caption-button glyphs on Windows 10
- tray and global hotkey support (kept in the host)

## 4. Working method

- **Who does what:** Opus orchestrates and writes the host and UI. Haiku subagents do the mechanical ports: lifting files into `dove-app`, converting themes to JSON, writing fixtures.
- **Commits:** one commit per screen or piece, only on `zgx/main` in `C:\dev\keplorayx-zgx`.
- **Source of truth for behaviour:** the original Dove folder, read-only. Zeron's zgx app is a reference for mechanisms only.
- **Builds:** one at a time in `C:/dvw/target-kx-zgx`.

## 5. Verification

- **Scripts:** `zgx test ui` (unit and keep-in-place identity tests), `zgx lint ui`, `zgx check ui`, and a design lint (`ZGX_DESIGN_LINT=error`) so no raw values reach screens.
- **Rust:** headless `TestApp` tests for the host bridge, plus `cargo test` across `crates/*`. The existing ~900 crate tests must stay green.
- **Screenshots:** offscreen audit shots of every gallery page and fixture screen, compared before and after each phase.
- **End-to-end:**
  - launch `keplorayx` with a scratch `DOVE_HOME`
  - drive it through the dove/cua MCP (AccessKit exposes a real UI Automation tree) and `zgx inspect`
  - run a fake-agent session: start a thread, stream a reply, approve a tool, open a diff, terminal and browser
- **Performance:** port `scripts/perf/perf.ps1` and `perf_check.ps1` to the new binary (frame stats from zgx in place of `DOVE_FRAME_STATS`). Run them side by side with the current Dove build as the baseline at every phase gate.
- **Isolation check:** after each session, `git -C <original Dove folder> status` shows clean, and `C:/dvw/target-ui-v4` has unchanged timestamps.