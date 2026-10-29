# Keplorayx UI/UX revamp v3: Zeron design parity, settings, motion, performance

Status: **proposal, awaiting approval** (2026-10-15). Nothing in the code has changed.
Earlier plan: [`docs/ui-revamp-plan.md`](../ui-revamp-plan.md) (v2, 2026-10-12). Most of its layout and feature work has shipped: top-bar thread tabs, chat ⇄ terminal, Kanban, Automations, the composer and the side panel. v3 does not redo that work. It brings Keplorayx to Zeron's design language and quality bar, rebuilds Settings, adds a motion system, and meets the performance targets.

Hard rules from the request, applied throughout:
1. **Nothing is removed.** Every feature, panel, setting, shortcut and workflow stays. Anything that would be merged or moved is listed in [§4 Decisions](#4-decisions-that-need-your-approval). Until you approve a proposal there, the default is to keep things where they are.
2. **Keplorayx keeps its identity:** its name, mark and branding, and the features Zeron lacks.
3. **Plan first.** Implementation starts only after you approve.

Screenshots are in [`shots/`](shots/). All Keplorayx captures come from an isolated QA copy (scratch `DOVE_HOME`, seeded threads); your real `~/.dove` was opened read-only for the copy. All Zeron settings captures come from an isolated instance (`ZERON_DATA_DIR` scratch).

---

## 0. Summary

**What Keplorayx is built on.** Keplorayx already runs on **ZUI**: it uses the same `zeronsh/zui` GPUI fork at the same revision (`361ff20`) as Zeron, plus `gpui-base`. No framework migration is needed. The gap is in the **component and design layer above ZUI**. Zeron's `crates/ui` is about 224k lines and includes `motion.rs`, `popover.rs`, `settings/widgets.rs`, `typography.rs`, the `theme` crate and a toaster. Keplorayx's UI is about 49k lines with hand-rolled primitives spread over three files. "Port ZUI components" therefore means porting those Zeron modules (MIT, already credited) into a single Keplorayx kit.

**Biggest gaps, in order:**
1. **Settings.** There are three columns (the app sidebar, a settings nav and the content). Every section opens with a hero card with badges. Labels wrap one word per line. Some icons are wrong. Labels are stale ("Voice & Jarvis", "Crew & Team"). Agent settings are split between Settings and the Agents page.
2. **No motion system.** Menus, popovers and panels appear instantly. Reduced motion is not supported. Four animations animate layout (height, margin, width).
3. **No type, spacing or radius scale.** There are 508 `text_size(px(..))` calls across 17 sizes and 15 different radii, and no UI font-size setting.
4. **Fragile error and empty states.** Diffs on a non-git folder dumps raw `git diff` usage text that overflows its panel and paints over the chat ([shot](shots/dove-diffs-error-overflow.png)).
5. **Idle and startup cost.** An idle window re-renders about once a second, forever, and uses 3–6% of a CPU core. At launch Keplorayx spawns about 10 agent CLIs to refresh model lists, a transient load of about 0.5 GB of child processes.

**Recommended path:** fix the performance issues first (no visual change), then tokens, motion and the kit, then rebuild Settings, then the shell chrome, chat and pages. Each step ships on its own.

---

## 1. Phase 1: Audit of Keplorayx

### 1.1 Stack

| Concern | What Keplorayx uses | Where |
|---|---|---|
| UI framework | GPUI from `zeronsh/zui` @ `361ff20` (backdrop blur, edge fades, rounded clip) + `gpui-base` (patched: the caret stops blinking after about 10 s) | `Cargo.toml`, `vendor/README.md` |
| Rendering | GPUI's immediate-mode element tree. GPU-composited quads, text and images. Blur through zui. No webview for the main UI | `apps/dove/src/ui.rs` (`frosted`, `rounded_clip`, `bottom_faded`) |
| Embedded web | Native web views: one WebView2 environment per window, created lazily per browser tab or canvas design (WKWebView on macOS, WebKitGTK on Linux) | `apps/dove/src/browser/*` |
| State | One root `Shell` entity owns child entities: Sidebar via a render fn, `ThreadView` per opened thread → `TaskView` (`Chat` rows) + `Composer`, `SidePanel`, `SettingsPage`, pages. Globals: `Theme`, `Toasts`, `Remote`. Settings live in a SQLite key-value table (`Store`), except Integrations, which use `integrations.json` | `shell.rs`, `thread.rs`, `transcript.rs`, `dove-store` |
| Engine | `dove-core::Engine` on a 2-worker tokio runtime. `start_task` → worktree → `dove_harness::spawn(SessionSpec)` → `LiveSession { events: flume, commands }`. The recorder appends JSONL and updates SQLite, then forwards to the UI. The UI drains on wake with an 8 ms coalesce | `crates/dove-core/src/lib.rs`, `transcript.rs:1281` |
| Harnesses | Zeron's vendored drivers (Claude, Codex, Cursor, OpenCode, Pi, Devin, Grok, Hermes, Antigravity, Kilo), with ACP for the rest. On Windows each spawn goes through a job object and a suspended "escrow" copy of `dove.exe` | `crates/dove-harness`, `vendor/zeron-harness/src/process/windows/escrow.rs` |
| Agent tools | Each harness gets the `dove` MCP server (`dove --mcp-browser`, stdio → loopback control) | `mcp_browser.rs`, `browser/control.rs` |

**Does Keplorayx use ZUI?** Yes, fully at the framework level. It does **not** reuse Zeron's component layer. A "migration to ZUI" therefore means:

| Zeron module (MIT) | Keplorayx today | Action |
|---|---|---|
| `crates/ui/src/motion.rs` (specs, curves, hover fades, reduced motion, pulse clock) | none (8 ad-hoc `with_animation` uses) | Port whole |
| `popover.rs` (Popup lifecycle with exit animation, anchored menus, nested menus, keyboard nav, modal, dialog parts) | `ui.rs` + `surface_chrome.rs` (two `menu_row`s), custom overlays | Port and replace |
| `settings/widgets.rs` (page column, section, card, row, toggle, select, option cards, segmented with indicator, actions, strips, tooltip, scroll fade) | `kit.rs` (switch, segmented, field, group) | Port and replace |
| `typography.rs` (rem scale, font families and sizes) | fixed px everywhere | Port; codemod call sites |
| `theme.rs` derivations (dialog and overlay mixes, `element_active`, `accent_wash`, contrast enforcement) + `crates/theme` (30 themes, VS Code import) | `theme.rs`, a subset of Zeron's fallback values; 1 theme × 5 accents | Port derivations now; theme library later (optional) |
| `toast.rs` (stacked toaster, swipe to dismiss, 4 s) | `kit.rs::Toasts` (5 s, max 4) | Port |
| `loaders.rs` pulse clock (30 Hz lease, pauses in background) | spinners animate every frame and re-render the Shell | Port the clock |

### 1.2 UI surface inventory

| Surface | Contents today | Key files |
|---|---|---|
| **Top bar** (36 px) | Sidebar toggle, back and forward (in a pill when the sidebar is closed). Thread tabs (40–210 px each; icon-only when the panel is open). `+`. Chat ⇄ Terminal segmented control. Thread-info popover. Panel toggle. Windows captions | `shell.rs:1490–1800` |
| **Sidebar** (264 px default, 208–400) | New thread (Ctrl N). Nav: Threads (count), Kanban, Automations, Pull requests, More ▸ (Agents, Usage, Archived, Settings, Help). Team section (+ Add a teammate). Needs-you group. Threads list (search, options menu: group by project, compact rows, show archived, add project). Two-line rows: status, agent mark, title, project · branch, time, pin. Footer: Local · host, today's tokens, files, settings | `sidebar.rs` |
| **Chat** | Transcript (virtualised `ListState`, 320 px overdraw). User bubbles with edit and copy. Agent header. Folded "Thought process · ran N commands…". Markdown (tables, code, Mermaid, math, images). Tool rows with tree rail. Diffs inline. Permission cards (A/S/D keys). Questions (1–9). Todo panel. Prompt rail (minimap). Jump to latest. Working trailer with rolling words and timer | `transcript.rs`, `tool_calls.rs`, `markdown*.rs`, `widget.rs` |
| **Composer** | Context row: project, device, checkout (local, worktree, cloud "coming soon"), branch, folders. Input with @ files and / commands, attachments, paste and drop. Mic (dictation). Permission mode (Ask, Auto-edit, Plan, Bypass with confirmation). Agent · model · effort picker. Persona. Send, Steer, Queue, Stop. Drafts saved per thread | `composer/mod.rs`, `composer/picker.rs` |
| **Right dock** (460 px default, 360–760) | Tabs from a `+` menu: Browser, Terminal, Canvas, Diffs, Files, History, Plan, Subagents, Chats, Side chat. Expand and close. Diffs: stage, commit, PR footer. Files: tree + preview + editor + office viewers | `side_panel.rs`, `changes.rs`, `files_panel.rs`, `history.rs`, `browser/`, `terminal/`, `canvas.rs`, `side_chat.rs`, `office/` |
| **Home / new thread** | Wallpaper hero with effect. Project and device pickers. Composer. Checkout, branch and permission chips. "N agents ready" updates pill | `home.rs`, `wallpaper.rs`, `updates.rs` |
| **Pages** | Threads (search, filters, bulk actions), Kanban (5 columns), Automations (+ Inbox), Pull requests (list + detail), Agents (paths, sign-in terminals), Usage (CSV export), Archived, Help, Team (roster + editor) | `pages/*`, `pull_requests.rs`, `team.rs` |
| **Settings** | A route inside the shell. Left nav (232 px, 4 groups, 13 sections) with a nav-only filter. A hero card per section. Card rows. Saves immediately | `settings.rs`, `pages/integrations.rs`, `keymap.rs` |
| **Modals and menus** | 25+ custom overlays: sidebar More and options menus, thread context menu (Rename, Pin, Archive, Copy ▸, Delete…), thread-info card, command palette (Ctrl K; actions, threads, projects, agents, files), filter menus, panel `+` menu, composer menus (persona, checkout, device, project, branch, mode, mic, attach, folders, @ and /), model picker, Bypass confirmation, mid-turn switch sheet, prompt-rail hover card, sign-in terminal sheet, toasts, intro splash, native OS pickers | inventory in §1.2a |
| **Empty, loading and error states** | About 90 strings across every surface. Spinner only, no skeletons. A blank transcript while loading. Raw error text in Diffs (overflows the panel) | inventory in §1.2a |

**§1.2a Full inventories.** The exhaustive tables are kept in [`inventory.md`](inventory.md):
- all 44 settings controls, 17 settings written elsewhere and 8 hidden keys;
- 17 rebindable shortcuts, 9 fixed bindings and 20 context keys;
- every modal and menu, and every empty, loading and error state.

These tables are also the baseline for the "nothing removed" checklist (§3.8).

### 1.3 Animation inventory

**The image animations** are in the new-thread backdrop. They are enabled by **Settings › Appearance › New-Thread Backdrop › "Animate backdrop"** (key `wallpaper_animate`, default off, needs a background image). **"Backdrop shader effect"** (`wallpaper_effect`) picks the effect:

| Effect | Motion | How it runs today | Cost |
|---|---|---|---|
| Original | Ken Burns push, 20 s loop | GPU paint transform, **but refreshes the whole window at 30 Hz** | Full-window frame 30×/s |
| Dither | Aircraft and contrails, brightness waves, 24 s loop | **CPU raster of the full hero every frame at 24 fps**, new atlas image per frame, drawn twice | About 72 MB/s GPU upload at 1×, about 290 MB/s at 2×; animated state 14–57 MB |
| Halftone | Dot ripple from the focal point | CPU raster at 15 fps | Same pattern |
| Scanlines | Rolling lines, sweep band, flicker | CPU raster at 24 fps + opacity flicker | Same pattern |

Animation stops when the home page is not drawn. It does not stop when the window is minimized or inactive. Animated state (`ANIM`) stays in memory after you leave home. The static `CACHE` (4 rasters) is never released.

**Other animations:**
- Spinners, 750 ms, opacity only. A running sidebar row re-renders the **whole Shell every frame**.
- Sidebar row glide, 260 ms; it animates **margin**.
- Tool-row reveal, 360 ms (**height**), and fold open and close, 140 ms (**height**).
- Activity shimmer, 3.4 s. It uses `frame_pump`, which notifies the TaskView every frame.
- Rolling text, 550 ms; it animates label **width**.
- Glide-to-row scroll, 18 × 14 ms timer steps.
- Browser agent pulse, 1.1 s.
- Intro, 3.85 s, first launch only.
- Reduced motion is never set: `cx.set_reduce_motion` has no callers.

### 1.4 Baseline performance

Measured on this machine: Windows 11, 125% scaling, 16 GB RAM, other agents running. The numbers come from the **debug** build (`target-wallpaper`, app at opt-level 1, deps at opt-level 2), because no release build exists. Release numbers will be lower; the first implementation step re-baselines in release. Method: `scratchpad/bench.ps1` launches a QA copy minimized, times process start to visible window, restores the window without stealing focus, samples private and working set and CPU, and reads Keplorayx's own `DOVE_FRAME_STATS` render counter.

| Scenario (Keplorayx debug) | Start → window | Main WS / Private | Idle CPU (1 core) | Shell renders/s idle |
|---|---|---|---|---|
| Home, dither still, first 16 s | 1.2–2.0 s | 98–109 / 106–136 MB (peak WS 164–174 during wallpaper decode) | 5.6% | 2–5 |
| Home, steady (after 25 s) | — | 98 / 136 MB | **2.8%** | **~1, forever** |
| 1 open thread (150 turns, rendered) | 1.5–1.8 s | 80 / 102 MB | 5.5% | 1–6 |
| 5 open threads | 1.6 s | 83 / 117 MB | 6.4% | 1–6 |
| 10 open threads, steady (after 25 s) | 1.1–1.6 s | 91 / 154 MB | **3.7%** | **~1, forever** |
| 10 threads + Diffs panel | 1.5 s | 83 / 106 MB | 6.7% | bursts of 14–27 |
| Settings page | 1.6–2.6 s | 67–73 / 85–130 MB | 0.8–12% | 1–13 |
| Your live Keplorayx (debug, home, minimized) | — | 144 / 167 MB | 2.3% (while minimized) | — |
| Zeron 0.2.x release (live, minimized), for reference | — | 203 / 149 MB | 6.4% | — |

Child processes Keplorayx starts **without any session**, sampled in the first 15 s after launch:

| Process | Why | Size |
|---|---|---|
| `npx -y amp-acp` (node), `droid exec --output-format acp-daemon` | model-catalog refresh starts full ACP agents | node 122 + 36 MB, droid 137 MB |
| `kilo`, `codex`, `cursor-agent --version`, `agy`, `amp`, `droid`, `hermes`, `kimi`, `powershell` | version probes and catalog refresh, one after another | kilo 136 MB, powershell 78 MB, … |
| 2× `dove.exe` (suspended) | Windows "escrow" helper per spawned agent (`zeron-harness`) | 13–20 MB each |
| Startup CPU | — | about 100% of a core for about 2 s |

The transient child load is about **0.5 GB** with no session running.

**Concurrent sessions (1, 5 and 10 with mixed harnesses).** Each open thread above is a real `ThreadView` with transcript, composer and list state, so the RAM figures cover Keplorayx's UI side of N threads. Not measured yet:
- **Live streaming CPU.** It needs N agents actually streaming.
- **The per-session processes** (agent CLI, escrow `dove.exe`, `dove --mcp-browser` bridge).
- **Input latency.** There is no instrumentation yet.

Doing it now would mean spending real agent tokens and typing into QA windows on a shared machine. **Step M0** below builds a replay harness: a fake agent CLI that streams recorded transcripts in Claude stream-json, Codex app-server and ACP formats. It measures these for 1, 5 and 10 sessions with no token cost, and adds a keypress-to-frame latency trace. A final run with real harnesses follows, with your OK.

**Top measured or traced causes** (details in §3.6): the 700 ms model-catalog poll in every composer (the ~1/s idle renders); spinners re-rendering the Shell every frame; CPU wallpaper rasters; the startup catalog spawn storm; closed tabs keeping their whole `ThreadView`; O(rows) scans on every streamed batch.

---

## 2. Phase 2: Zeron vs Keplorayx

Reference shots: Zeron [home](shots/zeron-home-live.png), [Settings › General](shots/zeron-settings-general.png), [Appearance](shots/zeron-settings-appearance.png), [Shortcuts](shots/zeron-settings-shortcuts.png), [Providers](shots/zeron-settings-providers.png). Keplorayx [home](shots/dove-home-live.png), [thread](shots/dove-thread.png), [Settings › General](shots/dove-settings-general.png), [Appearance](shots/dove-settings-appearance.png), [Keyboard](shots/dove-settings-keyboard.png), [Agents](shots/dove-settings-agents.png), [Threads page](shots/dove-threads-page-empty.png), [Diffs error](shots/dove-diffs-error-overflow.png).

### 2.1 Layout and information architecture

| Region | Zeron | Keplorayx | Verdict |
|---|---|---|---|
| Titlebar | 38 px. Left: sidebar glyph, back, forward, `+` new. Centre: harness icon + session title. Right: surface tabs (24 px chips) + files and right-pane toggles. 36 px captions | 36 px. Left pill: toggle, back, forward. **Thread tab strip** (Dove-only, kept). Right: Chat/Terminal segmented, info, panel toggle | Keep tabs. Adopt Zeron metrics and fix tab truncation |
| Sidebar | 256 (242–400). "All projects" filter. Pinned group. Single-line rows (status dot, harness mark, title, time). Archived shelf. Custom sections. Footer: Local, mic, PRs, settings | 264 (208–400). New-thread button. Nav block (5 rows). Team. Needs you. Two-line rows. Footer: device + usage | Keep Keplorayx's nav, Team and Needs you. Restyle rows to Zeron's grammar. Single-line density is an option |
| Chat column | Width 736 (560–1200, a setting). Composer max 768, radius 26. Message rail hidden below 768 px | Composer radius 20. Width not configurable | Adopt metrics; add a width setting |
| Right dock | Host 520 (360 to viewport − 300). Surface-picker cards. Docked Files column 220–440. Expand/takeover. **LRU column eviction** when narrow | 460 (360–760). `+` menu with 10 tab kinds. Expand. Fixed centre minimum | Adopt picker cards, eviction and slide transitions. Keep all 10 tab kinds |
| Settings | Route. **App sidebar is replaced** by "← Back" + section nav. 760 px page column. Title, then section labels, then cards of 60 px rows | Route. App sidebar **plus** settings nav **plus** content (3 columns). Hero card per section. Nav-only filter | Rebuild (§3.3) |

### 2.2 Visual language

| Aspect | Zeron | Keplorayx | Gap |
|---|---|---|---|
| Typography | Inter / JetBrains Mono. **rem-scaled** (UI size 12–20 px setting). Roles: page 20/26 medium, row title 13/17 medium, description 12/16, markdown 14/22, code 12.5/18. Weights mostly Medium | Inter / JetBrains Mono. **Fixed px**. 17 sizes (most often 12, 13, 11.5, 12.5, 11). No line-height roles. No size setting | High |
| Spacing | `SPACE_XS/SM/MD/LG` 4/8/12/16. Settings: section gap 32, row padding 16×12, min height 60 | Magic numbers (8, 12, 6, 10, 13, 14, 4, …) | High |
| Colour (dark) | bg #060606, surface #0d0d0d, raised #343438. Dialog and overlay mixes. `element_active` = accent α0.18. `accent_wash`. Contrast-checked muted text. Accent violet #8b7cf6 | Same base values (ported from Zeron's fallbacks). Missing dialog and overlay mixes, `element_active`, `accent_wash` and contrast enforcement. Default accent sky | Medium |
| Colour (light) | #ffffff / #f3f3f5, text #303035, accent #5b43e8 | Light exists; parity unverified | Medium |
| Themes and accents | 30 built-in themes, VS Code import, 7 accents + "theme default", match wallpaper colours | 1 theme, 5 accents | Medium (additive) |
| Radii | Control 6, panel 10, card 12, menu item 7, palette item 10, bubble 16, select 8, composer 26 | 15 distinct values; bubble 18, composer 20 | Medium |
| Shadows and borders | Glass plate y1 b2 α.22 + lift. Popover `shadow_lg` when opaque. 1 px borders. Focus ring 2 px accent | Ad hoc rings and shadows | Medium |
| Icons | Solar linear set, 16 px nav | The same Zeron icon files (copied). **Some wrong mappings**: Appearance shows a magnifier; Agents and Team share the bot glyph | Low |
| Density | Settings rows 60 px; sidebar rows single-line | Settings rows 70–90 px + hero; sidebar rows two-line (~56 px) | Medium |

### 2.3 Interaction

| Aspect | Zeron | Keplorayx |
|---|---|---|
| Hover | 150 ms colour fade (`EASE_TAILWIND`) on every interactive wash | Instant colour swap |
| Focus | 2 px accent ring. Arrow, Home and End keys in settings nav and menus. Typeahead filter in menus | Keyboard nav in the palette, model picker and transcript. Most menus and the settings nav are mouse-only |
| Active and selected | `element_active` accent tint; selected chips get a 1 px inset ring | `selected_bg` + ring |
| Menus and popovers | In 140 ms (opacity 0.3→1, −2 px rise); out 100 ms with exit animation; nested menus | Appear and disappear instantly; no exit |
| Panels | Width tween 180–240 ms; edge "nudge" bounce at limits | Instant |
| Dialogs | 180 ms fade + 2 px settle; scrim | Instant |
| Tabs and segments | Indicator slide 260 ms; tab slide 150 ms | Instant |
| Collapse | 180 ms (height) | 140 ms (height) |
| Reduced motion | System, On or Off + pause in background; reads the OS flag | None |
| Perceived speed | Zeron is not free either: 6.4% of a core while minimized | Idle ~1 render/s; startup child-process storm; debug build ~1.5 s to window |

### 2.4 Settings UX

| Aspect | Zeron | Keplorayx |
|---|---|---|
| Structure | 9 sections, grouped by spacing: General, Appearance, Notifications, Voice, Shortcuts · Providers, Devices · Files, Archived sessions | 13 sections under 4 headers: App · Agents & Studio · Workspace · System |
| Navigation | The sidebar becomes the settings nav ("← Back", Esc closes and returns to the previous route). Remembers the last section. Deep link `settings/<slug>` | A second nav column beside the app sidebar. Deep link `DOVE_SETTINGS` |
| Search | None | Filters nav sections only (label, blurb and keywords); row contents are not searched |
| Grouping | Page title, then a muted section label, then a 12 px-radius card, then rows with dividers | Hero card (icon, blurb, badge), then section label, then cards |
| Row layout | Title 13 medium + description 12 muted on the left (min width 160), control on the right, wraps cleanly | Label and hint on the left, control on the right. **Label column collapses** when the control is wide (one word per line) |
| Controls | Toggle 44.8×28.8 with 180 ms thumb travel. Select 32 px (popover menu). Option cards with previews. Swatches. Slider. Text actions in 4 tones | Switch, segmented control, swatches, outline buttons, text fields |
| Saving | Immediate for discrete choices; text and sliders debounced 400 ms; atomic JSON write; applied live | Immediate; text fields write on every keystroke and on blur; applied live |

### 2.5 Keplorayx's biggest UX gaps, ranked

1. **Settings IA and layout:** three columns, hero noise, broken row layout, stale labels, wrong icons, agent settings split across two places.
2. **No motion language:** no hover fades, menu or dialog transitions, panel slides, indicator slides or reduced-motion support. Some animations animate layout.
3. **No design-token discipline:** ad-hoc font sizes, spacing and radii; no UI scale setting. This shows as inconsistent rhythm on every page.
4. **Error and empty states:** Diffs stderr overflow, a blank loading transcript, no skeletons, raw error strings.
5. **Idle and startup cost:** ~1 render/s forever, Shell repainted every frame while any thread runs, CPU wallpaper rasters, the startup spawn storm.
6. **Top-bar tabs** degrade to unreadable icon-only chips when the dock is open.
7. **Keyboard reach in menus and settings:** no arrow-key navigation, typeahead or visible focus ring.
8. **Popover consistency:** two menu implementations, no exit animation, and placement differs per menu.
9. **Right dock:** a 10-item `+` menu instead of a picker; no responsive eviction, so the chat gets squeezed.
10. **Memory hygiene:** closed tabs keep their transcript, composer poller, web views and terminals until archive.
11. **Theme breadth:** 1 theme, 5 accents; no font-size or conversation-width controls.

What Keplorayx has that Zeron lacks, kept and restyled: thread tabs, Kanban, Automations, Team, Canvas, side chat, Usage, the Threads page, chat ⇄ terminal, office viewers, Integrations, remote mobile, and settings search (to be upgraded).

---

## 3. Phase 3: Revamp plan

### 3.1 Design system

**Tokens** go in a new `apps/dove/src/design/`, ported from Zeron with Keplorayx's identity preserved:
- **Type roles in rems:**
  - `page` 20/26 medium, `title` 15/20 medium;
  - `row` 13/17 medium, `body` 13/18, `meta` 12/16, `micro` 11/14, `badge` 10.5;
  - `md` 14/22, `code` 12.5/18, `diff` 12/21, `term` 13.
- **UI scale:** `window.set_rem_size` driven by a new "Interface font size" setting (12–20, default 16 = today's look).
- **Spacing:** 2, 4, 8, 12, 16, 24, 32 (`XXS`…`XXL`).
- **Radii:** control 6, menu item 7, select 8, panel 10, card 12, bubble 16, composer 26, pill 999.
- **Elevation:** glass plate (y1 b2 α.22 + y2 b6 α.06), popover `lg`, dialog scrim (α.60 dark / .32 light).
- **Colour:** add Zeron's derived tokens: `surface_dialog`, `surface_overlay` mixes, `element_active`, `accent_wash`, `*_muted` and `ensure_contrast` for muted text. Keep Keplorayx's accent list, keep **sky** as Keplorayx's default accent (identity), and add Zeron's extra presets (Amber, Cyan, Blue) as options.

**Components** go in a single kit (`apps/dove/src/kit/`) replacing `kit.rs`, `ui.rs` widgets and `surface_chrome.rs` duplicates:

| Component | Source | Notes |
|---|---|---|
| Button (Solid, Outlined, Filled, Quiet, Danger), IconButton (tooltip + shortcut), TextAction | Zeron `widgets.rs::action_button`, `popover.rs::btn_*` | Hover fade, focus ring |
| Toggle | Zeron `toggle_switch` (44.8×28.8, 180 ms thumb) | Replaces `kit::switch` |
| Select (builder) | Zeron `select()` / `Select<V>` | Replaces most segmented controls with more than 3 options |
| Segmented + IndicatorSlide | Zeron `section_tab`, `IndicatorSlide` | Theme, surface, mode |
| OptionCard (with preview), Swatch, Slider | Zeron `option_card`, `accent_swatch`, appearance slider | Colour scheme cards; conversation width |
| TextField, SearchField | `gpui-base` input + Zeron `search_input_frame` | Debounced commit |
| Menu, NestedMenu, ContextMenu, Popover | Zeron `popover.rs` (`Popup<T>`, `anchored_menu_*`, `menu_row`, `menu_step`, `filter_indices`, `HoverIntent`) | One implementation for all 25+ overlays |
| Modal, Dialog, Sheet | Zeron `modal`, `dialog_*` | Bypass confirmation, mid-turn sheet, delete confirmations |
| Tooltip | Zeron `TextTooltip` | — |
| Toaster | Zeron `toast.rs` | Keeps today's toast messages |
| PageColumn, Section, Card, Row, RowTile, Badge, Kbd, ErrorStrip, WarningStrip, ScrollFaded | Zeron `widgets.rs` | Settings, pages, panels |
| EmptyState, **Skeleton** (new), Spinner and StatusGlyph on a pulse clock | Keplorayx `kit.rs` + Zeron `loaders.rs` pulse | Skeleton is new for loading transcript, diff and PRs |
| ResizeSeam (20 px hitbox, edge nudge) | Zeron `resize_drag_sample` | Sidebar and dock |
| SurfacePicker cards | Zeron right-pane picker | Dock `+` |

### 3.2 Per-surface redesign

Every row ends with **Features kept**. Nothing is dropped; anything that would move is listed in §4.

**Top bar**
- *Current:* 36 px. Pill with toggle, back and forward. Tab strip (40–210 px) that becomes icon-only when the dock opens. Chat/Terminal segmented control, info, panel toggle.
- *Proposed:*
  - 38 px Zeron titlebar metrics.
  - Tabs get a minimum readable width (status + 10 characters). Overflow collapses to a "+N" tab menu instead of shrinking to icons. The active tab shows a title tooltip.
  - When there is room for no tab at all, the centre shows the active thread title (Zeron's pattern).
  - Chat/Terminal becomes a segmented control with an indicator slide. Hover fades on every control.
  - Panel toggles become Zeron's files/right-pane glyphs, with a morph animation (`GLYPH_STATE` 280 ms).
- *Features kept:* sidebar toggle, back/forward, thread tabs (reorder, middle-click close, Ctrl 1–9, Ctrl Tab), `+`, Chat ⇄ Terminal, info popover (fork, side chat, open folder, terminal, copy path), panel toggle, captions.

**Sidebar**
- *Current:* New-thread button, nav block, More menu, Team, Needs you, Threads list (search, options), two-line rows, footer.
- *Proposed:*
  - Zeron row grammar: 32–36 px rows, status dot, harness mark, title, time on hover or always, pin, archive on hover.
  - Project and branch move to a second line only in "Comfortable" density. The existing `sidebar_density` setting becomes Comfortable (today's two-line) or Compact (single-line, Zeron).
  - "Pinned" group header, archived shelf as a disclosure (Zeron), Needs-you rows keep inline Allow/Deny.
  - Nav rows and the More menu use the kit menu (keyboard navigable).
  - Spinner for running rows on the pulse clock, so the Shell stops re-rendering every frame.
  - Row resort glide uses an offset transform, not margin.
- *Features kept:* everything listed in §1.2, including the options menu (group by project, compact, show archived, add project), search, attention sort, context menu (Rename, Pin, Archive, Copy ▸, Delete), drag reorder, Team, and the footer device and usage.

**Chat**
- *Current:* virtualised transcript, folded activity, markdown, tool tree, permission and question cards, todos, rail, working trailer; composer radius 20.
- *Proposed:*
  - Zeron chat metrics: markdown 14/22, code 12.5/18, bubble radius 16, composer radius 26 and max width 768.
  - Conversation width setting (560–1200, default matches today's look).
  - Activity folds reveal content by opacity and offset. Height is set once, not tweened. The shimmer runs on the pulse clock.
  - Skeleton while a transcript loads (instead of blank).
  - Permission and question cards on the kit Card with Zeron's button tones.
  - Error rows become an ErrorStrip with Retry.
  - Message rail restyled to Zeron (tick 10 px, max 12 ticks, glide 500 ms).
- *Features kept:* every row type, edit and resend, copy, fork, A/S/D and 1–9 keys, Alt ↑/↓, PageUp/PageDown, Ctrl Home/End, rail, jump-to-latest, todos, working timer, generative UI cards, Mermaid, math, images.

**Composer**
- *Proposed:*
  - Zeron composer plate (`input_bg`, radius 26, 76–260 px grow).
  - Context chips on the kit Select.
  - Permission-mode menu with descriptions on the kit Menu.
  - Model picker keeps Keplorayx's picker, restyled to Zeron's compact picker spacing (rows 32 px).
  - Send button state morph (Send, Steer, Queue, Stop) uses a 150 ms crossfade.
- *Features kept:* all chips and menus in §1.2, @ and /, attachments, paste and drop, mic, persona, Bypass confirmation, drafts, steer and queue.

**Right dock**
- *Current:* 460 px, `+` menu of 10 kinds, expand, close.
- *Proposed:*
  - Zeron surface host: default 520, minimum 360, maximum viewport − 300.
  - `+` opens a **surface-picker card grid** (Browser, Terminal, Diffs, Files, History, Canvas, Plan, Subagents, Chats, Side chat) with keyboard navigation.
  - Opening and closing slides the panel by offset + opacity over 180 ms. The chat column snaps to its final width once at the start, so nothing reflows on every frame.
  - Responsive fit: when the window narrows, the least recently opened column (sidebar or dock) hides and keeps its width, then returns when there is room. This is Zeron's `horizontal_panel_fit`; it hides, never closes.
  - Proper error and empty states: Diffs on a non-git folder shows "Not a git repository" with "Initialize git" or "Open folder" instead of raw stderr, and all panel content is clipped.
- *Features kept:* all 10 tab kinds and their contents (stage, commit, push, PR footer, comments, file tree and editor, office viewers, history, browser with agent control, terminals, canvas bridge, plan, subagents, chats, side chat), expand, resize and persisted width.

**Home**
- *Proposed:* Zeron hero spacing and composer plate; updates pill restyled to Zeron's harness-updates popover (list width 360, rows 64); wallpaper revamp in §3.5.
- *Features kept:* project, device, checkout, branch and permission pickers, the updates pill and wallpaper effects.

**Settings:** see §3.3.

**Pages** (Threads, Kanban, Automations, Pull requests, Agents, Usage, Archived, Help, Team)
- *Proposed:*
  - Zeron page column (header 20/26, subtitle, sections, cards).
  - Filters on the kit Select. Tables in Card rows with 40 px row height.
  - Empty and error states with consistent icon, title and body.
  - Kanban cards on the kit Card with drag lift (shadow + opacity).
  - Lists over about 200 items use `uniform_list` (Threads, Archived, Automation runs).
- *Features kept:* every filter, bulk action, column, editor, export and empty state listed in the inventory.

**Modals, menus, palette**
- *Proposed:*
  - All overlays move to the kit Popup: in 140 ms, out 100 ms, keyboard navigation, typeahead, Esc.
  - Command palette restyled to Zeron's `palette_card`, and it also indexes settings rows ("Settings: Reduce motion").
  - Toasts move to Zeron's toaster.
- *Features kept:* every menu item listed in the inventory.

### 3.3 Settings rebuild (mirrors Zeron)

**Shell.**
- Opening Settings swaps the app sidebar for the settings nav: "← Back" at the top, then section rows with 16 px icons, grouped by spacing as in Zeron.
- Esc or Back returns to the previous route. The last section is remembered. `DOVE_SETTINGS=<slug>` and palette deep links keep working.
- Content sits in a 760 px page column: page title, muted section labels, then cards of rows (title 13 medium + description 12 muted on the left, control on the right).
- The hero cards go away. Badges such as agent counts move into the nav, as today.
- Settings stay a route (as in both apps), not a window.

**Search (better than Zeron's).**
- The "Filter settings…" field at the top of the nav becomes a **row-level** search.
- Results list matching rows across all sections ("Appearance › Reduce motion"). Enter jumps to the row and flashes it for 900 ms.
- The palette indexes the same rows.

**Saving.**
- Discrete controls save and apply immediately, as today.
- Text fields commit on a 400 ms debounce or on blur (today: every keystroke).
- Storage keys do not change. Every existing key is read and written as before, so there is no migration and rollback is safe.

**Navigation** (proposed; the moves marked † need approval in §4):

| Group | Section (slug) | Icon |
|---|---|---|
| 1 | General (`general`), Appearance (`appearance`), Notifications (`notifications`), Voice (`voice`), Shortcuts (`keyboard`, alias `shortcuts`) | settings, tuning, bell, microphone, keyboard |
| 2 | Agents (`agents`, Zeron's "Providers"), Team (`team`)†, Canvas (`canvas`), Integrations (`integrations`), Remote (`remote`, Zeron's "Devices") | widget, users-group, gallery, widget-add, smartphone |
| 3 | Data & Storage (`data`), Archived (`archived`)†, About (`about`) | database, archive, info |

**Every existing setting mapped to its new home:**

| # | Setting (key) | Today | New location | Control |
|---|---|---|---|---|
| 1 | Reopen thread tabs on launch (`restore_tabs`) | General › Threads & Startup | General › Threads | Toggle |
| 2 | Prewarm agent harness while typing (`prewarm`) | General › Threads & Startup | General › Threads | Toggle |
| 3 | Isolate each new thread in a git worktree (`use_worktree`) | General › Git & Worktrees | General › Git & worktrees | Toggle |
| 4 | Branch prefix (`branch_prefix`) | General › Git & Worktrees | General › Git & worktrees | Text, debounced |
| 5 | Default clone directory (`clone_dir`) | General › Git & Worktrees | General › Git & worktrees | Path + Browse… |
| 6 | Default permission level (`permission_mode`) | Composer | General › Composer† (or Composer if not approved) | Select |
| 7 | Projects authorized for Bypass (`bypass_ok.*`) + Revoke all | Composer | General › Composer† | List + Quiet action |
| 8 | Colour theme (`appearance`) | Appearance › Theme & Materials | Appearance › Color scheme | 3 option cards with previews |
| 9 | Accent colour (`accent`) | Appearance | Appearance › Color scheme | Swatches (5 today + 3 added) |
| 10 | Window surface material (`surface`, legacy `frosted`) | Appearance | Appearance › Material and background | Select: Glass, Acrylic, Normal |
| 11 | New-thread background image (`wallpaper_path`) | Appearance › Backdrop | Appearance › Material and background | Tile + Change/Remove, or Choose image |
| 12 | Backdrop effect (`wallpaper_effect`) | Appearance › Backdrop | Appearance › Material and background | Select (disabled with a hint until an image is set) |
| 13 | Animate backdrop (`wallpaper_animate`) | Appearance › Backdrop | Appearance › Material and background | Toggle |
| 14 | Per-agent Enabled/Hidden (`agent_disabled.<id>`) | Agents | Agents › list row | Toggle; "Not installed" state |
| 15 | Sign-in & custom paths →, Configure Harnesses | Agents | Agents › row chevron and header action (opens the Agents page detail) | Row drill-in |
| 16 | Persistent specialist notes (`team_notes`) | Crew & Team | Team | Toggle |
| 17 | Open Crew Page | Crew & Team | Team › "Open Team page" | Action |
| 18 | Open designs automatically (`canvas_auto_open`) | Canvas | Canvas | Toggle |
| 19 | Interactive 2-way agent bridge (`canvas_bridge`) | Canvas | Canvas | Toggle |
| 20 | Default artboard viewport (`canvas_default_device`) | Canvas | Canvas | Segmented |
| 21 | Local Parakeet model (install, progress, cancel, remove, retry) | Voice | Voice › Dictation | Status row + actions |
| 22 | Microphone (`voice_device`) | Voice | Voice › Dictation | Select |
| 23 | 17 rebindable shortcuts (`key.<id>`) + Rebind/Reset/Reset all | Keyboard | Shortcuts, grouped (Navigation, Threads, Panels, App) + "Restore defaults" header action | Binding chip (click to record) |
| 24 | System desktop notifications (`notifications`) | Notifications | Notifications › Desktop | Toggle |
| 25 | Completion sound (`notification_sound`) | Notifications | Notifications › Sounds | Toggle |
| 26 | Keep running in background (`keep_running`) | Notifications | Notifications › Background | Toggle |
| 27 | Enable remote control (`remote_enabled`) | Remote Control | Remote | Toggle |
| 28 | Woozlit account sign in/out, connection status, device ID, last error | Remote Control | Remote | Rows + actions |
| 29 | Keplorayx for Chrome: status + pairing code copy | Integrations | Integrations › Chrome | Row + copy |
| 30 | Composio: enabled, API key, toolkits, other slugs (`integrations.json`) | Integrations | Integrations › Composio | Toggle, masked field, chips, text |
| 31 | Paper: enabled + status | Integrations | Integrations › Paper | Toggle + status |
| 32 | Computer use: enabled, status, Check, Detect again, install guide | Integrations | Integrations › Computer use | Toggle + actions |
| 33 | Storage size + Open folder | Data & Storage | Data & Storage | Row + action |
| 34 | Storage path (DOVE_HOME) + Copy | Data & Storage | Data & Storage | Row + copy |
| 35 | Telemetry & privacy ("Zero telemetry") | Data & Storage | Data & Storage | Badge row |
| 36 | Version, License, Source repository | About | About | Rows + action |
| 37 | Sidebar: group by project (`sidebar_group`), compact rows (`sidebar_density`), show archived (`sidebar_archived`) | sidebar options menu only | **stays in the menu** + mirrored in General › Sidebar (additive) | Toggles |
| 38 | Agent browser tools (`agent_browser`), office skills (`agent_office_skills`) | no UI (hidden keys) | General › Agent tools (additive, proposed) | Toggles |
| 39 | Agent binary paths (`agent_path.*`), sign-in | Agents page | unchanged (Agents page); linked from Settings › Agents | — |
| 40 | Layout state (`sidebar_open`/`_width`, `panel_*`, `open_tabs`, `active_project`), drafts, picker memory, caches | implicit | unchanged (implicit state, no UI) | — |
| 41 | Archived threads | More › Archived page | **stays** + new Settings › Archived entry that shows the same page† | — |

**New settings** (additive, each listed in §4):
- Appearance › Motion: Reduce motion (System, On, Off) and Pause animations in background.
- Appearance › Fonts and layout: Interface font size, Code font size, Terminal font size and Conversation width.
- Later: Light and dark theme selectors (Zeron's theme library).

### 3.4 Motion system

Port Zeron's `motion.rs` as `apps/dove/src/motion.rs`.

| Token | Duration | Easing | Use |
|---|---|---|---|
| `HOVER_FADE` | 150 ms | (0.4, 0, 0.2, 1) | every hover, press and selection wash |
| `MENU_IN` / `MENU_OUT` | 140 / 100 ms | ease (0.25, 0.1, 0.25, 1) | popovers and menus: opacity 0.3→1, 2 px rise; exit mirrors |
| `DIALOG_IN` | 180 ms | ease | modals and sheets: opacity + 2 px settle; scrim fade |
| `PANEL` | 180–240 ms (√distance) | ease | sidebar and dock slide (offset + opacity) |
| `TAB_SLIDE` | 150 ms | ease-out | tab reorder |
| `INDICATOR_SLIDE` | 260 ms | (0.2, 0, 0, 1) | segmented and nav selection indicator |
| `GLYPH_STATE` | 280 ms | ease-out-quint | sidebar and panel glyph morph |
| `LIST_REORDER` | 260 ms | ease-out-quint | sidebar resort, Kanban drop: offset glide from old to new slot |
| `ENTER` (`FADE_IN`) | 500 ms | ease-out-expo | first appearance of a page or row: opacity + 4 px rise |
| `FADE_QUICK` | 150 ms | ease | crossfades (send-button state, chevrons 200 ms) |
| `NEW_THREAD` | 420 ms | ease-out-quint | home → thread handoff |
| `SCROLL_GLIDE` | 500 ms | ease-in-out | rail jump, scroll-to-row |
| Pulse clock | 33 ms tick (30 Hz), 15 Hz "slow" | — | spinners and activity; leased, so it stops when no one is pulsing |

**Rules:**
- **Only opacity and position offset (translate) are animated, plus GPU paint transforms on images and SVG.** Width, height, margin and padding are never tweened. GPUI is immediate-mode: any animated element re-runs layout for its own view on each frame, and there is no compositor-only layer. So the rule is enforced in practice:
  - Animated elements are kept in small isolated views or entities, so a frame touches only that subtree.
  - Size changes happen once at the start, followed by an opacity and offset reveal.
- **Today's layout animations get converted:**
  - sidebar glide (margin) → offset;
  - tool-row reveal and folds (height) → final height at once + content fade and rise;
  - rolling text (label width) → fixed-width slot + crossfade;
  - panel width → slide.
- **Reduced motion:**
  - System, On or Off. On Windows it reads `SPI_GETCLIENTAREAANIMATION`; on macOS, `accessibilityDisplayShouldReduceMotion`; on Linux, the portal setting.
  - The result is applied through `cx.set_reduce_motion`, so `with_animation` snaps.
  - Hover fades, indicators and toggles snap. Spinners become a subtle 2.4 s opacity pulse.
  - "Pause animations in background" stops every loop while the window is inactive or minimized.
- **Zero cost at rest:** no loop runs unless something is visibly animating. A CI check uses `DOVE_FRAME_STATS`: 0 renders/s after 2 s idle. A `DOVE_MOTION_SCALE` debug knob is ported to slow animations down for review.

### 3.5 Backdrop (image) animation revamp

**Goals:**
- Look better.
- A steady 60 fps, or 120 fps on 120 Hz displays.
- **Zero CPU and GPU** when disabled, offscreen, minimized or inactive.

**Approach: precomputed layers + GPU transforms, no per-frame raster.** zui has no custom-shader hook, so each effect is decomposed into a few static textures rendered once on the background executor, then animated only by paint transforms and opacity:

| Effect | Precomputed once | Animated per frame (GPU only) | Visual upgrade |
|---|---|---|---|
| Original | the image (cover-cropped) | Ken Burns zoom and pan via paint transform (already GPU) | eased drift, slight parallax between two crops; **no full-window refresh**, only the hero view repaints |
| Dither | dithered base; a pre-dithered aircraft sprite + contrail strip in the image hues; 2–3 dithered "wave" highlight masks | aircraft along an eased, banking path (translate + rotate on the sprite); contrails as a strip revealed by a moving clip with fading opacity; waves as mask offset + opacity | smoother flight, contrails that dissipate, no 24 fps stutter |
| Halftone | halftone base; a halftone "ring" band sprite | ring scaled outward from the focal point (image paint transform) with opacity falloff, staggered rings | soft ripple, no 15 fps stepping |
| Scanlines | base with scanline pattern; a tiling line texture; a soft band sprite | line texture rolled by offset; band swept by offset; flicker by opacity | CRT roll + vignette, smooth |

- **Frame pacing:** driven by `request_animation_frame` (display vsync), not 24 or 15 fps timers. With no raster, a frame is a few transform updates. Target in release: < 1% of a core at 60 Hz, < 2% at 120 Hz, and 0 bytes of per-frame texture upload.
- **Stopping:** the loop runs only while the hero is drawn **and** the window is visible and active (pause-in-background) **and** reduced motion is off. Leaving home releases `ANIM` frames and the `CACHE` rasters; only one still is kept. Old atlas images are dropped with `Window::drop_image`.
- **Fallback:** if a layer can't express an effect faithfully, render a short looping set of keyframes at half resolution once and crossfade between them, still with no per-frame raster.
- **Optional later:** a small zui patch adding a shader-quad primitive for full-resolution GPU effects. It is fork maintenance, so it is not in scope unless you want it.

### 3.6 Performance

**Targets** (release build; harness child processes reported separately):

| Metric | Baseline (debug) | Target |
|---|---|---|
| RAM, Keplorayx's own processes, 10 concurrent sessions (main + escrow helpers + MCP bridges + mesh host + web views) | main 80–154 MB with 10 threads open; per-session helpers not yet measured live | **≤ 300 MB** with no browser tabs open; **≤ 500 MB** with 3 active browser tabs; **1 GB hard ceiling** enforced by a watchdog that hibernates hidden web views and trims caches |
| Idle CPU | 2.8–6.7% of a core; ~1 render/s forever | **0 renders/s and < 0.1% of a core** after 2 s idle |
| CPU while streaming | not yet measured | 1 session < 5% of a core; 10 sessions < 20%; background (unviewed) sessions cause 0 renders; no polling |
| Startup | 1.1–2.0 s to window (debug); about 10 child CLIs in the first 15 s | **≤ 400 ms** to window, **≤ 600 ms** to an interactive home (warm); **no child processes in the first 5 s** |
| Input latency (keypress → frame presented) | not yet instrumented | p50 ≤ 8 ms, p99 ≤ 16 ms (one 60 Hz frame) while 3 sessions stream |

**Causes and fixes:**

| # | Cause (evidence) | Fix | Expected effect |
|---|---|---|---|
| P1 | Model-catalog **700 ms poll in every Composer**, forever (home + each opened thread) — `composer/picker.rs:408–430` | Event-driven: the engine notifies when a catalog changes; no timer | Idle renders → 0 |
| P2 | Running spinners in the sidebar **re-render the Shell every frame**; the activity shimmer `frame_pump`s the TaskView every frame — `kit.rs:185`, `tool_calls.rs:232` | Spinners and shimmer in tiny isolated elements on the leased 30 Hz pulse clock; paused when the window is inactive | Streaming CPU sharply lower (Shell no longer rebuilt at 60–120 Hz) |
| P3 | Wallpaper CPU raster at 15–24 fps + full upload each frame; Original refreshes the window at 30 Hz; `ANIM`/`CACHE` never released — `wallpaper.rs` | §3.5 | Home idle → ~0; 14–57 MB freed off-home |
| P4 | **Startup spawn storm:** catalog refresh starts ACP agents (node, droid), plus ~8 `--version` probes — `dove-core lib.rs:1030–1110`, `shell.rs:391–410` | Use cached `models.v2.*`; refresh one agent lazily when its picker opens, or once a day in idle time, sequential and low priority. Cache versions keyed by binary mtime and size; probe only when the Agents page or updates pill is shown | ~0.5 GB transient and ~2 s of a core removed from launch |
| P5 | Each agent spawn on Windows creates a **suspended full `dove.exe` escrow** (13–20 MB) — `zeron-harness …/escrow.rs` | Point the escrow at a tiny stub exe (`dove-escrow.exe`, about 100 KB) shipped next to Keplorayx; record the patch in `vendor/README.md` | ~15 MB → ~1 MB per session |
| P6 | `dove --mcp-browser` bridge per session runs the full GUI binary | Measure first; if it is heavy, build the bridge as a small separate binary without GPUI | Up to ~10–15 MB per session |
| P7 | **Closed tabs keep their `ThreadView`**: transcript, composer + poller, web views and terminals until archive — `shell.rs:901–913` | Hibernate on close after a 30 s grace: drop transcript rows, composer and list state when no agent is live (reloaded on reopen). Terminals and live agents keep running. Web views: see §4 D7 | RAM scales with *visible* threads, not history |
| P8 | O(rows) work per streamed batch: `sync_panel` clones todos and scans subagents and designs; `pending()` and `user_messages()` per render — `shell.rs:817–848`, `transcript.rs:757, 3052` | Maintain incremental indexes inside `Chat::apply` | Batch cost O(new events) |
| P9 | Transcript memory: full JSONL replay into memory; assistant text kept twice; full tool outputs; unpruned subagent events — `transcript.rs:354–381`, `dove-store lib.rs:673` | Drop the duplicate `messages`; keep tool output heads and tails in memory with the full text on disk (offset index); prune subagent events after the mirror; **tail-first load** (last ~64 entries first, older ones hydrated on scroll) | Long threads open faster; RAM per thread bounded |
| P10 | Quadratic `TextDelta` merge (`format!`) — `transcript.rs:1336` | `String::with_capacity` + `push_str` | Faster large bursts |
| P11 | Settings read from SQLite **on every frame** (home: 3, sidebar: 1) through one mutex — `home.rs:55–57`, `sidebar.rs:218` | Settings cached in a Global, updated on write | Lower frame cost, no lock contention |
| P12 | Integrations view polls `cx.notify` every 2 s, created at startup — `integrations.rs:237` | Event from `browser::control` on extension connect or disconnect; create the view lazily | Removes a forever timer |
| P13 | Unbounded caches: Mermaid SVGs, GPUI `img()` cache, canvas docs — `markdown_extra.rs:19`, `canvas.rs:286` | LRU caps (e.g. 64 diagrams, 128 MB images); drop offscreen images | Bounded long-session RAM |
| P14 | Terminal scrollback 10,000 lines per terminal (up to ~30 MB each at wide widths) — `emulator.rs:42` | Keep the default; add a "Scrollback lines" setting (additive) | User control |
| P15 | Remote publish reads the whole transcript up to every 600 ms (only when remote is on) — `remote.rs:289–311` | Incremental publish of new events | Remote-on streaming cost lower |
| P16 | Debug numbers today | All acceptance numbers in a `--release` build with `mimalloc` | Honest benchmarks |

**Measurement harness (M0)** in `scripts/perf/`:
- **`dove-fake-agent`:** replays recorded transcripts as Claude stream-json, Codex app-server JSON-RPC or ACP at realistic token rates. It is registered through `agent_path.*` in a scratch `DOVE_HOME`, giving 1, 5 and 10 concurrent mixed-harness sessions with no token cost.
- **`perf.ps1`** (built from today's `bench.ps1`):
  - startup: process start → window → first frame, with a `DOVE_FRAME_STATS` marker;
  - RAM split: Keplorayx main, escrow, bridges, web views and mesh host vs harness children;
  - idle CPU and streaming CPU.
- **`DOVE_INPUT_TRACE`:** timestamps at key dispatch and at the next presented frame (`on_next_frame`), giving a p50/p99 histogram.
- **Final real run** with Claude, Codex and Gemini sessions, only with your OK, because it spends tokens.

### 3.7 Migration strategy

Each milestone is a separate branch and PR from a **git worktree**, because the shared checkout is edited by other agents. Keplorayx builds, runs and passes CI (`clippy -D warnings`) at every step.

| Step | Scope | Ships as | Risk |
|---|---|---|---|
| **M0** | Prerequisites: finish the in-progress merge (see Risks), release baseline, perf harness, auto-generated feature inventory (routes, setting keys, keybindings, palette actions, menus, panel kinds), reference screenshot set | tooling only | low |
| **M1** | Performance quick wins: P1, P2, P4, P5, P10, P11, P12 | invisible | low |
| **M2** | Design tokens + typography roles (rem) + Zeron colour derivations; mechanical codemod of 508 `text_size` calls to roles (Haiku-assisted, reviewed) | near-invisible (default scale = today) | medium (layout shifts; caught by screenshots) |
| **M3** | Motion system port, reduced motion, hover fades, conversion of layout animations | visible polish | low |
| **M4** | Kit consolidation (§3.1), migrating surfaces one module at a time | visible polish | medium |
| **M5** | **Settings v3** behind `DOVE_SETTINGS_V3` / setting `settings_v3` until the checklist passes, then the default; old page code deleted one release later (code only; every setting already lives in v3) | big visible win | medium |
| **M6** | Shell chrome: titlebar and tabs, sidebar rows, right-dock picker, slide and responsive fit | visible | medium |
| **M7** | Chat and composer metrics, skeletons, error and empty states (incl. the Diffs fix) | visible | medium |
| **M8** | Pages, modals, palette, toaster | visible | low |
| **M9** | Backdrop animation revamp | visible | medium |
| **M10** | Memory: tab hibernation, transcript caps, tail-first load, LRU caches, web-view hibernation | invisible | medium–high (state restore) |
| **M11** | Optional: Zeron theme library (30 themes, VS Code import), extra fonts | additive | low |
| **M12** | Acceptance: screenshots, checklist, benchmarks | report | — |

After each step I post a short summary (what changed, numbers, screenshots) and what comes next.

**Risks:**
- **The checkout is mid-merge *and* mid-rebase.** `MERGE_HEAD` and `REBASE_HEAD` exist. Nine files are unmerged (including `settings.rs`, `shell.rs`, `composer/mod.rs`, `voice.rs`, `integrations.rs`), with markers removed but not committed. That work belongs to another session; it must be finished before M1.
- **Concurrent editors.** Other agents (including one that has twice saved stale buffers over files) work in this checkout. Mitigation: own worktree and branch per milestone, small PRs, line-count and marker checks before each commit.
- **GPUI has no scale/rotate transform for divs.** The motion rules use offset + opacity (Zeron does the same); image and SVG transforms are available for the backdrop.
- **rem migration** may shift layouts. Mitigation: default 16 px base = today; screenshot diffs per surface.
- **Web-view hibernation** loses in-page state (form input, scroll); this needs your decision (§4 D7).
- **Vendored harness patch** (escrow stub) must be re-applied on vendor updates; it will be documented in `vendor/README.md`.

**Rollback:**
- Every milestone is an isolated, revertable commit or PR.
- Settings v3 sits behind a flag until accepted.
- Motion can be globally disabled (Reduce motion = On).
- No storage key is renamed or removed, so old builds read the same `~/.dove`.
- Perf changes are separate commits with before and after numbers.

### 3.8 Acceptance criteria

1. **Visual parity screenshots:** Keplorayx vs Zeron side by side for home, thread, right dock (Diffs, Files, Browser, Terminal), sidebar (comfortable and compact), top bar (tabs, narrow), every settings section, menus and popovers, dialogs, empty, loading and error states.
   - Dark + light, at 100%, 125% and 150% scaling.
   - Plus a token check script: sampled font sizes, row heights, radii and colours match the token table.
2. **Nothing-removed checklist:**
   - The auto-generated inventory from the baseline commit (routes, 44 settings controls + 17 elsewhere + 8 hidden keys, 17 + 9 + 20 shortcuts and keys, palette actions, 25+ menus and their items, 10 panel kinds, page features) is diffed against the final build. Every item is ticked with its new location.
   - A manual workflow script: new thread, send, steer and queue, approve (A/S/D), question (1–9), chat ⇄ terminal, side chat, fork, rename and pin and archive and delete, Kanban drag, automation run, PR flow (stage → commit → push → PR), browser agent control, canvas, voice dictation, remote toggle, settings round-trip.
3. **Benchmarks** in the release build via `scripts/perf`: a table against every target in §3.6 for 1, 5 and 10 sessions (fake-agent replay), plus one real mixed-harness run. Harness child RAM is reported separately. An idle-renders check is added to CI.

---

## 4. Decisions that need your approval

Defaults if you don't approve: keep things where they are and skip the additive extras.

| ID | Proposal | Why | If not approved |
|---|---|---|---|
| D1 | Settings **replaces the app sidebar** with "← Back" + section nav (Zeron), instead of showing both | Removes the cramped 3-column layout. The sidebar returns on Back or Esc; nothing is lost | Keep the app sidebar visible; restyle the settings nav as a second column |
| D2 | Fold the **Composer** section (default permission level, Bypass list) into **General › Composer** | Zeron has no Composer section; it holds two rows | Keep Composer as its own section |
| D3 | Rename **"Crew & Team" → "Team"** (and remove "Crew" wording on the Team page and in the blurb) and **"Voice & Jarvis" → "Voice"** | Your earlier "Team, never Crew" decision; Jarvis code was deleted | Keep the labels |
| D4 | **Settings › Archived** entry showing the same Archived page (also stays under More) | Zeron parity | Not added |
| D5 | Additive settings: Reduce motion, Pause animations in background, Interface, code and terminal font size, Conversation width, Terminal scrollback lines, 3 more accents, and toggles for the hidden `agent_browser` / `agent_office_skills` keys | Zeron parity and accessibility | Only Reduce motion (needed for the motion rules) |
| D6 | **Lazy model catalogs and version probes** (P4): use the cached list at launch, refresh when a picker opens or once a day | Removes the ~0.5 GB startup spawn storm | Keep the refresh at launch, but make it sequential and deferred by 10 s |
| D7 | **Web-view hibernation:** hidden browser tabs and canvases of *closed* thread tabs are destroyed after 10 min and reloaded (same URL) when shown | Bounds RAM under the 500 MB target | Keep web views alive until archive (today) |
| D8 | Sidebar density: "Compact" becomes Zeron's single-line row; "Comfortable" stays today's two-line row (the default remains yours to pick) | Zeron look without losing information | Restyle only, same density |
| D9 | Optional **M11**: Zeron's 30-theme library + VS Code theme import | Theme breadth | Skip |

---

## 5. What happens after approval

1. **M0:** coordinate with the session that owns the in-progress merge, build the perf harness and release baseline, and generate the feature inventory. I report the real 1, 5 and 10-session numbers before any UI change.
2. Proceed M1 → M12, with a short summary after each step.
