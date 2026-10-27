# Notch v2: the best Dynamic Island on macOS, Windows and Linux

Status: **V0–V7 and V10 done on Windows** (2026-10-27); **V8 (macOS) and V9 (Linux) not started**: they need those machines to build and test on. Decisions in §10.

- **V3 (zero-idle Windows layer):** `notch/win.rs` runs one hidden message window for raw-input pointer moves (coalesced to 60 Hz with a trailing timer), the clipboard listener, foreground WinEvent hooks (location changes only for the foreground process), the Ctrl+Alt+Space hotkey and display/setting changes. Media comes from GSMTC events (`notch/media.rs`), the volume meter from a COM audio thread. The loop sleeps on one channel or the next deadline; nothing polls. After 10 s hidden the window closes and the working set is trimmed.
- **V4 (design tokens + harness):** every size, colour, radius and spring in `notch/tokens.rs`; `scripts/notch_sheet.ps1` makes a contact sheet of every screen from the QA hooks.
- **V5 (motion):** `notch/spring.rs`, interruptible springs for the island's 7-value shape (fixed 1/240 s steps); the dot drop, lift on close, pointer magnetism, press squish, the header puck and staggered rises.
- **V6 (modules):** every screen in §5.4 (`notch/views.rs`): slim with stacked agent marks, rings, the Done moment and bubbles; banners with diff/command previews, Deny with Undo, A/D keys; the command bar; Home cards; grouped Agents with hover actions and the rate strip; the agent view (chips, light Markdown, steps, approvals, quick replies, @ attach, steer/queue); New agent; Music (art backdrop and tint, seekable scrubber, shuffle, scroll volume, output device); Tray (filters, typed tiles, paste back, drag out, capture); two-pane Notes with checklists, pinning and send as prompt; Timer (ring, presets, scroll to set, link to an agent); Notifications; Dev servers; Focus mode.
- **V7 (glass):** `notch/glass.rs` puts a Windows.UI.Composition host-backdrop blur under the GPUI surface, clipped to the island's rounded shapes; the captured GDI blur stays as the fallback (`DOVE_NOTCH_CAPTURED_GLASS=1` forces it), and Settings can turn glass off.
- **V10 (settings + perf):** Settings › Notifications › Notch gains notifications, glass, focus, display, slim activities and tabs; `scripts/perf_check.ps1 -Island` measures the closed-Dove cost.
- **Limits of this pass:** GPUI draws circular corners, so the island is not a true squircle; shared-element transitions are approximated by the puck, springs and rises; the island has no voice module; only files (not text or images) can be dropped on it; link tiles have no favicons.

- **V2:** the island is `dove --island` (the same executable in its own process, so packaging stays one binary; it loads no engine, webview or voice) in `apps/dove/src/island/`: `ipc.rs` (newline-delimited JSON over a per-data-folder named pipe / Unix socket, token in `<home>/run/island.token`; owning the pipe also makes the engine single-instance, so a second launch shows the first window), `host.rs` (engine side: snapshots pushed on attention changes, commands run through the window's Shell or straight on the engine when headless, sessions handed to and back from the closed window, the close/idle lifecycle with a 20 s grace), `mod.rs` (island main, one per data folder, the tray with Open Dove, Start at login and **Quit Dove completely** with a confirmation when agents are running), `login.rs` (HKCU Run key; never from dev builds or another `DOVE_HOME`). `dove --headless` runs the engine with no window. The notch now runs only in the island and reaches agents only over the pipe; when Dove isn't running it says so, keeps music/clipboard/notes/timer, and launches Dove (headless, or with its window for Open) when a command needs it.
  - Verified end to end on a scratch home: launch starts the island; snapshot over the pipe; a second launch forwards and exits; closing the window keeps the engine for the grace period, then it quits while the island stays; headless serves the pipe and `open` brings the window; `quit_all` stops the engine.
  - Not yet: the tray menu itself was not clicked in QA; agent team tools and the in-app browser need the window (headless agents get errors from those tools until it reopens); headless resumes use default run settings (Ask mode).
  - **Island cost before V3 (debug, hidden, alone):** 63 MB working set, 78 MB private, 1.5–1.8 % CPU. **After V3:** 2–5 MB working set, ~40 MB private, 0.16–0.31 % of one core (debug build, 60 s sample, measured 2026-10-27); a release build should be lower.

- **V0:** v1 re-measured (§1).
- **V1:** `dove-core/src/attention.rs` (`Attention`, `TaskState`, fed by the recorder for chat and terminal-mirror sessions, change pings without per-token wake-ups), `Engine::send/decide/interrupt` over a command line the engine keeps into every live session (the recorder now forwards commands while any sender is alive, not just the view's), and `apps/dove/src/attention.rs` (`Shell::task_state/task_pending/task_reply/task_send/task_decide/task_interrupt`, engine first, view as fallback, plus `watch_attention` so the sidebar repaints for threads with no view). The sidebar's Needs you, Voice Mode and the notch all moved onto it; the notch refreshes on attention pings instead of polling every 0.4 s.
Starts from: [NOTCH.md](NOTCH.md) (the v1 spec, and §0 there for what v1 shipped on Windows). Part of [ROADMAP.md](ROADMAP.md) §3.5.

v1 proved the idea on Windows: a frosted island that drops out of the top edge as a dot, a slim live-activity capsule, approvals, an agent view you can message, music, a clipboard and file shelf, notes and a timer. v2 has three goals:

1. **Make it feel like Apple made it.** A new visual system, interruptible physics, shared-element motion, and better modules.
2. **Make it ambient.** It keeps working with Dove's window closed (and with Dove itself quit), costs almost nothing while idle, and the tray quits everything.
3. **Make it native on all three OSes.** Real glass on each, the real notch on Macs, and layer-shell on Linux.

---

## 1. Where v1 stands (measured)

| | v1 on Windows (debug build) |
| --- | --- |
| Code | `apps/dove/src/notch/`: 6 files, ~4,100 lines; 14 unit tests pass |
| Process | Lives inside Dove. "Keep running" only **minimizes** the main window, so a "closed" Dove still holds the whole UI (~80–150 MB) and the tray polls every 300 ms |
| Data path | Reads agents through `Shell` → `TaskView` (approvals live in the transcript's rows), so the notch is dead without the main window |
| Idle cost, hidden | No measurable CPU change; +10–16 MB working set, ~+22 MB private (a second GPUI window) |
| Cost, open | 11–19 % of a core before the last optimization pass (full-size screen capture every 0.4 s, a 20 fps equalizer). **V0, after the fix** (GDI shrinks during the copy, 10 fps idle animation; 3 runs, debug build, `notch_bench.ps1`): notch off 61–67 MB working set / 86–87 MB private / 1.6–4.4 % CPU; hidden +6–11 MB working set, +~22 MB private, CPU unchanged, 1 render; expanded 80–92 MB / 106–114 MB / 3.2–7.2 % CPU, 10 renders while still. Release-build numbers are still to take (V2, with the island process) |
| Polling | Pointer, foreground window, clipboard and media are all polled on timers (50 ms – 1 s) |
| Glass | DWM acrylic ignores window regions, so v1 captures the screen behind and blurs it in-app. It looks right and stays readable, but it lags behind moving video and costs a capture several times a second |
| Platforms | Windows only; macOS and Linux keep it off |

What v1 got right and v2 keeps: the dot-to-island morph, the shape-clipped window (clicks around the island pass through), the Shell-equivalent actions (send = composer Send, safety-gated approvals), the dark frosted look, and the settings.

## 2. What "#1" means

Most rivals are music-plus-shelf utilities: NotchNook, Boring Notch, Alcove, DynamicLake, Droppy, Tinycast. Syllo adds coding agents, but watches them from outside through hooks. To lead, Dove needs to be the only one that is all of these:

| Claim | Proof we hold ourselves to |
| --- | --- |
| **Agent-native** | Approve, reply, steer, stop and start any of 20+ agents from the island, with the same safety rules as Dove. Approvals show a real diff preview |
| **Ambient and free** | Island-only process: ≤ 18 MB working set, ≤ 0.02 % CPU and ≤ 1 wake-up per second while idle. Nothing renders while hidden |
| **Native everywhere** | The real notch on Macs; real vibrancy/acrylic/KWin blur; Wayland layer-shell; multi-monitor |
| **Delightful** | 120 Hz interruptible springs, shared-element transitions, no dropped frames on a 2020 laptop |
| **Private** | Local only. Clipboard secrets never stored. Every module can be turned off |

## 3. Process model: works with Dove closed, costs almost nothing

### 3.1 Three pieces

```text
 ┌────────────── dove-island (always on, tiny) ──────────────┐
 │ tray / menu-bar item · notch window · pointer watcher     │
 │ music, clipboard, shelf, notes, timer (its own data)      │
 └───────────────▲──────────────────────────┬────────────────┘
                 │ attention snapshots      │ commands (send, decide,
                 │ + events (local IPC)     │ stop, start, open)
 ┌───────────────┴──────────────────────────▼────────────────┐
 │ dove (engine)  ── headless when no window is open         │
 │   Engine + AttentionModel + agent sessions + automations  │
 │ dove (UI)      ── main window, only while it's open       │
 └───────────────────────────────────────────────────────────┘
```

| Situation | What runs | Target |
| --- | --- | --- |
| Dove open | `dove` (UI + engine) + `dove-island` | as today, plus the island |
| Window closed, agents or automations running | `dove --headless` (engine only, **no GPU, no views**) + island | engine ≤ 60 MB; island ≤ 18 MB |
| Window closed, nothing running | `dove-island` only | ≤ 18 MB working set, ≤ 0.02 % CPU |
| Tray › **Quit Dove completely** | nothing | both exit; running agents are asked first |

- **`dove-island`** is a new small binary (`apps/island`), built like the `dove-mesh-host` sidecar: GPUI for the island only, no engine, no webview, no voice. It owns the tray (moved out of `notify.rs`) and starts at login (setting).
- **Closing Dove's window** drops the window and every view (Shell, TaskViews, panels, the browser). The engine stays only while something is running or scheduled; otherwise the process exits. The island then shows "Dove isn't running", and its New agent button starts `dove --headless` and hands it the prompt.
- **Opening Dove** (tray, island, shortcut) relaunches the UI or attaches a window to the headless engine.

### 3.2 Engine-level attention model (the prerequisite)

Today pending approvals and "send" logic live in `TaskView`. v2 moves them down so the engine alone can serve the island, the sidebar, the phone and Voice Mode:

| Move to `dove-core` | Today in |
| --- | --- |
| `AttentionModel`: per task its status, pending permission requests and questions, todos, last activity line, last reply tail, usage and rate-limit windows; changes published on a channel | `TaskView::voice_state`, `Chat::rows` |
| `Engine::send(task, text)`: steer when live, queue or resume when idle (the composer's Send) | `TaskView::submit` |
| `Engine::decide`, `Engine::answer`, `Engine::interrupt` | `TaskView::decide_from_outside`, `interrupt` |
| `Engine::start(new_task)`, with worktree creation | `Shell::notch_start` (copied from Voice Mode) |

Then the sidebar, Voice Mode, the phone remote and the island all read one model and can never disagree. The UI's `TaskView` becomes a consumer, not the owner.

### 3.3 IPC

- Transport: a named pipe (`\\.\pipe\dove-<user-sid>`) on Windows, a Unix socket (`~/.dove/run/island.sock`, mode 0600) on macOS and Linux. Length-prefixed JSON frames, the same style as the mesh sidecar.
- Auth: the OS restricts the pipe/socket to the current user, plus a random token in `~/.dove/run/island.token`.
- Engine → island: `Hello`, a full `Attention` snapshot, then **diffs** (`TaskChanged`, `ApprovalRequested`, `TaskFinished`, `UsageWindow`). No polling.
- Island → engine: `Send`, `Decide`, `Answer`, `Stop`, `Start`, `OpenThread`, `ShowWindow`.

### 3.4 Event-driven, zero-idle platform layer

| Signal | v1 | v2 (Windows) | macOS | Linux |
| --- | --- | --- | --- | --- |
| Pointer near the top | `GetCursorPos` every 50 ms | Raw input (`RIDEV_INPUTSINK`) on a message-only window: woken only when the mouse moves, coalesced to ≤ 30 Hz, nothing while still | Tracking area on the notch rect | Layer-shell input strip (Wayland), XInput2 raw motion (X11) |
| Full-screen app | Polled | `EVENT_SYSTEM_FOREGROUND` + `EVENT_OBJECT_LOCATIONCHANGE` WinEvent hooks (out of context) | `NSWorkspace` active-space notifications | `_NET_ACTIVE_WINDOW` property changes; on Wayland the compositor hides overlays itself |
| Clipboard | Sequence number every 400 ms | `AddClipboardFormatListener` | `NSPasteboard.changeCount` checked only while open, plus on app activation | `wl_data_device` selection events / XFixes selection notify |
| Media | Polled every second | GSMTC `SessionsChanged`, `PlaybackInfoChanged`, `MediaPropertiesChanged`, `TimelinePropertiesChanged` events | MediaRemote adapter notifications (§6) | MPRIS `PropertiesChanged` |
| Timer | Checked each tick | One wake scheduled at expiry | same | same |
| Tray | Polled every 300 ms | Event callback on the UI thread | `NSStatusItem` | StatusNotifierItem (`ksni`) |

While hidden for 10 s the island **closes its window** (freeing the swapchain and sprite atlas), drops album art and the glass frame, and trims its working set. Reopening a window measured in v1 is well under the 160 ms hover delay. A hover rebuilds it inside that delay.

## 4. Visual system

Designed directly in code (§10.5): each state is built, then reviewed from screenshots of the running build over light, dark and busy backgrounds.

### 4.1 Shape and material

- **Geometry.** Slim 36 px tall, a full capsule. Expanded corners 34–38 px using a **continuous (squircle) curve**: GPUI only draws circular corners, so add a superellipse path primitive to zui (or draw the island as a path) and use the same path for the window region and the hit test.
- **Floating.** 8 px below the top edge on Windows and Linux. On a notched Mac it is the notch: the top corners are concave "shoulders" that flow into the menu bar.
- **Glass (layers, bottom to top).** Live blur of what's behind (§6) → a tint (dark 64–76 % default; light and "clear" variants) → a 0.5 px inner specular highlight along the top edge → a 1 px hairline at 10 % white → content. A soft outer shadow on platforms that allow it outside the shape (macOS natively; Windows via a second, shadow-only layered window).
- **Art-aware tint.** While music plays, the glass picks up a faint glow from the album art's dominant colour. Agent alerts glow amber; failures red; finished green. Glows are 6–10 % strength, never neon.
- **Type.** Geist and Geist Mono (already bundled) with tabular numbers for timers, progress and counts. Six sizes only: 11, 12, 13, 15, 20, 40.
- **Iconography.** One stroke weight (1.6) everywhere; agent brand marks keep their colours.

### 4.2 Presentations (Apple's model, adapted)

| Presentation | When | Content |
| --- | --- | --- |
| **Minimal** | 2+ live activities | Each extra activity as its own small glass bubble beside the capsule (up to 2), with a tiny ring for progress |
| **Compact** | 1+ live activity | Leading: the primary activity's icon (agent mark with a todo-progress ring, album art, timer ring). Trailing: its state ("Needs you", elapsed time, an equalizer driven by the real audio level). Swipe sideways or scroll to cycle the primary |
| **Banner** | Approval, finish, failure, timer done | Grows to ~380 × 96: who, what, and the 2–3 actions. Never steals focus. Stays until handled for approvals; 5 s for the rest |
| **Expanded** | Hover, click or hotkey | The full island (§5) |

Users choose which activities may appear in Compact and their order (Settings › Notch › Live activities).

### 4.3 Motion

- **Interruptible springs** that keep velocity: a hover-out halfway through opening reverses smoothly instead of restarting. Critically damped for size, slightly under-damped (≈ 6 % overshoot) for the open.
- **Dot birth** stays: a 13 px drop from above the edge, then the spring. On Macs the island grows out of the hardware notch instead.
- **Shared elements:** an agent's mark in Compact flies to its row in Expanded; album art grows from 20 px to 72 px; the timer ring becomes the big timer.
- **Content:** rows fade and rise 6 px, staggered 18 ms; tab changes cross-fade with a 4 % scale; numbers roll.
- **Magnetism:** within 40 px of the pointer the capsule leans toward it by up to 3 px; a press squishes it 2 %.
- **Rates:** the display refresh rate only while something moves; 0 fps at rest; equalizer and pulses at ≤ 30 fps and only while visible. Reduced motion becomes cross-fades.

## 5. UX: the expanded island

### 5.1 Layout

- **Command bar first.** Typing as soon as the island opens (with the hotkey, focus goes straight in) searches threads, notes, clipboard and actions ("new claude task…", "timer 25", "note …"), the same engine as Dove's Ctrl K.
- **Home is a live dashboard of cards,** ordered by attention: needs you → working → now playing → timer → recent clips and notes. Each card expands in place.
- **Modules** sit in a compact segmented control: Agents · Music · Tray · Notes · Timer. Labels show on hover; numbers 1–5 switch. Modules can be reordered or hidden.
- **Keyboard everywhere:** ↑↓ to move, Enter to open, A / D for approvals, Esc to close, Ctrl+Alt+Space (rebindable) to toggle from any app.

### 5.2 Modules

**Agents** (the core):
- Rows with a todo-progress ring around the mark, a live ticker of the current tool ("Editing `auth.rs`"), elapsed time, and tokens/cost for the turn.
- The **approval card shows a diff preview** (file, +/− lines, the first hunk), or the command to be run, with Deny · Always · Allow. Risky requests show Review in Dove.
- An agent view with a streaming reply, **quick replies** ("Continue", "Run the tests", "Explain"), `@` to attach shelf items, push-to-talk via Voice Mode, Stop, and Open in Dove.
- Team runs grouped under their parent with fan-out progress; race groups with Compare (ROADMAP §3.2).
- Rate-limit meters with a one-time "plan running low" banner.
- Flock panes: send a line to the pane's PTY, with Enter confirmation.

**Music:**
- Art-tinted card, a scrubbable progress bar, volume (scroll on the island) and output device.
- The equalizer is driven by the real output level (Windows `IAudioMeterInformation` on the default endpoint, macOS a Core Audio tap, Linux PipeWire/PulseAudio monitor peak), sampled only while visible.

**Tray** (clipboard and shelf merged into one, with filter chips: All · Files · Text · Images · Links · Pinned):
- Smart previews: links with title and favicon, colours as swatches, code with language and syntax colours, image thumbnails.
- Drag out (Windows done; macOS `NSDraggingSession`; Linux XDND/`wl_data_source`); drop in text, URLs and images (not just files).
- Select and press Enter to paste into the app you came from.
- Send to agent attaches to the composer, not as a pasted path.
- Agents can read the tray through MCP (`tray_list`, `tray_get`; opt-in).

**Notes:**
- Markdown with checklists.
- Pin a note to Compact.
- "Send as prompt" starts or steers an agent with it.
- Notes sync across machines only if remote control is on.

**Timer:**
- Focus sessions with a ring in Compact.
- Link a timer to an agent ("tell me when it finishes or in 20 minutes, whichever first").
- Chime and banner at zero.

**New (from v1's extras list), each a module you can turn off:**
1. **Notifications:** one list of finishes, failures, PR checks, merges and automation runs. It replaces OS toasts while the island is on.
2. **Voice:** while a call runs, the island shows the orb and live waveform, with mute and hold-to-talk.
3. **Dev servers:** ports per worktree, with open, copy and stop.
4. **Screenshot to agent:** capture a region or window into the tray or straight into a thread.
5. **Focus mode:** only needs-you breaks through.

### 5.3 Small things that make it feel finished

- **Empty states that teach.** "Drop files here", "Copy something to see it here", "Start an agent", each with the shortcut.
- **Undo for destructive island actions:** Deny, delete a note, clear the clipboard.
- **Toasts inside the island** for "Sent", "Copied" and the like, never OS popups.
- **Accessibility:** UIA/AX names on every control, focus rings, a high-contrast solid material, and the reduced-motion and reduced-transparency OS settings respected.
- **Multi-monitor:** the island follows the display with the pointer (setting: primary, pointer, or all), per-monitor DPI, and moves below a top-placed taskbar.

### 5.4 Screen-by-screen redesign

§5.2 says what each module does; this says how each screen should look and feel. Every screen in v1 is rebuilt to these specs in V5 and V6, and reviewed from the contact sheet (V4).

**Slim (Compact): the signature view, so it gets the most care.**
- **Shape.** A 36 px capsule, 8 px below the edge, centered on the screen and not on its content: content grows outward evenly. Width snaps between three sizes (single 196, dual 264, wide 320) instead of tracking every glyph, so it never jitters as counts change.
- **Leading slot** (the primary activity):
  - An agent mark inside a 22 px progress ring (todo progress, or an indeterminate shimmer while it works).
  - Or album art at 22 px, softly rounded.
  - Or a timer ring.
- **Trailing slot:**
  - One short phrase in 12 px medium ("Needs you", "3 working", "4:12", "Done").
  - Or a real-level equalizer (4 bars, 2.5 px, rounded).
  - Never both.
- **Several agents.** At most 3 marks overlap like stacked avatars (−6 px), with a count chip ("+2") after them, not a row of icons and digits.
- **Needs you.** The capsule's hairline turns amber and breathes (1.6 s), and the leading mark gets a 6 px amber dot. Nothing else changes, so it reads at a glance without shouting.
- **Moments.** When a task finishes, the trailing slot rolls to a green "Done" with a check that draws itself (400 ms), then rolls back. When a track changes, the art cross-fades and the title marquees once in the trailing slot.
- **Bubbles (Minimal).** Extra activities are 36 px glass circles 6 px apart. They're born from the capsule's edge with a little stretch, like a droplet separating.
- **Idle with "keep visible" on.** The Dove mark and the time, at 60 % opacity, at single width.
- **Interaction.**
  - Hover lifts the capsule (+1 px, a stronger highlight) before it expands.
  - Scroll over it cycles the primary activity.
  - Middle-click plays or pauses music.
  - Right-click opens a small menu: Hide for 1 hour, Settings.

**Banner (approvals, finishes, failures, timer).**
- About 380 × 96, with the agent mark at 32 px in a tinted plate.
- Two lines: the title in 13 semibold; then what it wants, in 12, with the command or file in Geist Mono.
- Actions sit right-aligned as pills, primary last (Deny · Always · Allow). Keys A and D work while it's showing.
- Risky requests show an amber shield icon and only "Review in Dove".
- A thin countdown line along the bottom for timed banners.

**Header (expanded).**
- One row: module tabs as an icon segmented control with a sliding glass "puck" for the selected one, labels on hover, then the command bar field (it opens wide when you type), then `+` and Pin.
- Toasts appear as a pill that slides out of the header, not text squeezed between buttons.

**Home.**
- A two-column card grid on wide content, one column otherwise.
- Cards (radius 22, inner glass at 6 %) in attention order: Needs you, then the working agents, Now playing, Timer, Recent clips, Pinned note.
- Each card has a header row (icon, label, count) and expands in place with the spring.
- Empty: a centered Dove mark with three suggestion chips (Start an agent · Play music · New note).

**Agents list.**
- Rows grouped under sticky headers: Needs you · Working · Recent · Flock.
- Each row (52 px):
  - The mark inside its progress ring.
  - The title in 13 medium, and under it a live ticker of the current step in Geist Mono 11 that slides up as it changes.
  - On the right: elapsed time, plus the turn's cost in faint text.
- Hover reveals inline actions (Message · Stop · Open) instead of only a status word.
- A rate-limit strip at the bottom: one thin meter per agent kind, which turns amber at 80 %.

**Agent view.**
- **Hero row:** the mark in its ring, the title, status, branch and worktree as chips, then Stop and Open.
- **Body** is a mini transcript, not a plain text box:
  - The latest reply with light markdown (bold, code spans, lists).
  - Tool steps folded into one line ("Edited 3 files · ran tests ✓").
  - The approval card inline, with a diff preview (file name, +12 −3, the first hunk in mono with green and red gutters).
- **Composer:** pinned at the bottom with quick-reply chips above it, `@` to attach from the Tray, a mic for push-to-talk, and Send. The placeholder says whether a message will steer or queue.

**Media player.**
- The card's background is the album art, blurred and darkened, under the glass, so every song tints the island.
- Art at 88 px with a soft shadow and 18 px corners; it scales down 4 % while paused.
- Text: title in 15 semibold (marquee if long), artist in 12, the source app with its icon in faint 11.
- **Scrubber:** 4 px, thickening to 8 px on hover, draggable, with the elapsed and remaining times in tabular figures.
- **Controls:**
  - Previous, play/pause and next, with play/pause as a 48 px white circle that morphs between its two icons.
  - Shuffle and like where the player supports them.
  - Volume by scroll, with a slim vertical meter that appears while scrolling.
  - An output-device chip.
- **Nothing playing:** recent sources as chips that open the player.

**Tray (clipboard and shelf).**
- Filter chips at the top (All · Files · Text · Images · Links · Pinned).
- A masonry grid of tiles, not a list:
  - Images as thumbnails.
  - Links with favicon and title.
  - Code with language and syntax colours.
  - Colours as swatches.
  - Files with their real shell icon and size.
- Tiles lift on hover and show actions (Copy · Send to agent · Pin · Remove). Drag any tile out.
- Dropping on the island expands it with a dashed "drop here" glow that follows the pointer.

**Notes.**
- A list on the left and the editor on the right when expanded wide; stacked when narrow.
- Markdown with live checkboxes. The first line becomes the title.
- Pinned notes get a small pin badge and can show in Compact.
- "Send as prompt" sits on the editor's toolbar.

**Timer.**
- A large ring (120 px) with the time centered in Geist 40 tabular figures. The ring drains smoothly, and the last 10 s pulse.
- Presets as a horizontal chip rail, and custom time by scrolling the digits.
- Start is a pill that becomes Pause in place. A "linked to agent" chip shows when a timer is tied to an agent.

**Everywhere.**
- 8 px spacing grid; corner radii 30 (island), 22 (cards), 14 (rows), 999 (pills).
- One accent per meaning: amber for needs you, blue for working, green for done, red for failed.
- Hover is a wash plus a 1 px lift; a press squishes 2 %.
- Every list scrolls with edge fades. Loading uses shimmering skeletons, never a blank island.

## 6. Per-platform native layer

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Placement | Top-center of the chosen monitor, under any top taskbar | **Inside the hardware notch**: `NSScreen.safeAreaInsets`, `auxiliaryTopLeftArea/RightArea`; non-notch screens get the floating capsule under the menu bar | Wayland layer-shell (`Overlay`, top anchor, exclusive zone 0) on KDE, Hyprland and Sway; X11 `_NET_WM_WINDOW_TYPE_NOTIFICATION` + above; GNOME: under the top bar, XWayland fallback |
| Window | Popup, `WS_EX_NOACTIVATE`/`TOOLWINDOW`, shape via window region | Borderless `NSPanel` (non-activating), level above the menu bar, `canJoinAllSpaces` + `stationary` + `fullScreenAuxiliary` (setting) | Layer surface, input region = island |
| **Glass** | Spike first: a Windows.UI.Composition `DesktopWindowTarget` under GPUI's DirectComposition target, with a host-backdrop brush clipped to the squircle geometry. That is real-time acrylic that respects the shape. Fallback: v1's in-app capture blur | `NSVisualEffectView` (`.hudWindow`, behind-window blending) with a mask image of the shape: real vibrancy | KDE `org_kde_kwin_blur` with the island region; Hyprland/Sway blur via the layer namespace `dove-island`; GNOME: the solid material |
| Focus for typing | Take the foreground only when a field is clicked; hand it back on close (v1) | `NSPanel` becomes key without activating Dove | Layer-shell keyboard interactivity `on_demand` |
| Media | GSMTC events | MediaRemote via the `/usr/bin/perl` adapter (entitled) with an AppleScript fallback for Spotify and Music | MPRIS (`zbus`) |
| Audio level | `IAudioMeterInformation` | Core Audio process tap (14.4+), else none | PipeWire/PulseAudio monitor peak |
| Drag in/out | Done (out); add an `IDropTarget` for text/URLs/images | `NSDraggingDestination` / `NSDraggingSession` | XDND + `wl_data_device` / `wl_data_source` |
| Tray and login | `tray-icon`, Run key | `NSStatusItem`, `SMAppService` login item; the island is an `LSUIElement` helper inside `Dove.app` | `ksni` StatusNotifierItem, XDG autostart |
| Hotkey | `RegisterHotKey` (exists) | Carbon `RegisterEventHotKey` | Portal `GlobalShortcuts`, `XGrabKey` |

## 7. Performance and resource budgets (CI-gated)

| Measure | Budget |
| --- | --- |
| `dove-island` idle, hidden (release) | ≤ 18 MB working set, ≤ 25 MB private, ≤ 0.02 % CPU, ≤ 1 wake-up/s |
| `dove-island` with Compact showing music | ≤ 35 MB, ≤ 0.3 % CPU |
| Expanded, still | 0 renders/s; glass refresh only on change |
| Expanded, animating | Display rate, 0 dropped frames (2020 i7 laptop) |
| `dove --headless` with 3 agents | ≤ 60 MB plus the agents' own processes |
| Hover → first frame | ≤ 50 ms after the 160 ms intent delay, window recreated included |
| Battery | No measurable drain at idle versus the island off (macOS Energy Impact "Low", Windows E-cores) |

`scripts/perf_check.ps1` gains `-Island`. A macOS script uses `footprint` and `powermetrics`; Linux uses `/proc` and `perf stat`. All three run in CI on release builds.

## 8. Phases

Each phase ships on its own. Sizes: **S** days, **M** 1–2 weeks, **L** 3+ weeks.

| Phase | Contents | Size | Done when |
| --- | --- | --- | --- |
| **V0: Baseline** | Re-measure v1 after the capture/animation fix; release-build numbers; record the budget gaps | S | A table of real numbers in this doc |
| **V1: Engine attention model** | §3.2: `AttentionModel`, `Engine::send/decide/answer/interrupt/start` in `dove-core`; TaskView, sidebar and Voice Mode move onto it; tests | M | The notch works with no TaskView open for the thread |
| **V2: Process split** | §3.1/§3.3: `apps/island` binary, IPC, `dove --headless`, window-close lifecycle, tray moved to the island with **Quit Dove completely**, start at login | L | Close Dove's window: the island keeps showing and approving agents; with nothing running only the island stays, within budget |
| **V3: Zero-idle Windows layer** | §3.4: raw-input pointer, WinEvent hooks, clipboard listener, GSMTC events, close-when-hidden, working-set trim | M | Hidden-island budgets met on Windows |
| **V4: Design tokens** | §4 and §5 as code: colour, type, radius, spacing and motion tokens for the island, plus a screenshot harness that renders every state over light, dark and busy backgrounds | S | Every state captured in one contact sheet |
| **V5: Visual system and motion** | Squircle path (zui), glass layers, art tint, presentations, interruptible springs, shared elements, magnetism; the slim view, banner and header rebuilt to §5.4 | L | Every state reviewed in the contact sheet; 0 dropped frames |
| **V6: Modules v2** | Every screen rebuilt to §5.4 (Home, Agents list, Agent view, Media player, Tray, Notes, Timer). Command bar, Home cards, Agents (diff previews, quick replies, @ attach, rings, ticker), Music (scrub, volume, real meter), Tray (merged, smart previews, paste-back), Notes, Timer, Notifications, Voice, Dev servers, Screenshot, Focus | L | Every module in §5.2 usable by keyboard and pointer |
| **V7: Real glass** | Windows composition-backdrop spike → ship or keep the fallback; macOS vibrancy; KWin blur | M | Glass follows moving video at display rate on Windows and macOS |
| **V8: macOS** | §6 macOS column: notch integration, NSPanel, helper app, MediaRemote adapter, drag, login item | L | Daily-drivable on a notched MacBook and on an external display |
| **V9: Linux** | §6 Linux column: layer-shell (KDE, Hyprland, Sway), X11, GNOME fallback, MPRIS, SNI tray | L | Works on KDE Wayland, GNOME Wayland, X11 |
| **V10: Polish and gates** | Accessibility, multi-monitor, settings page "Notch" of its own, perf gates in CI on all three, docs | M | All budgets green in CI |

Order (§10.5): V0 → V1 → V2 → V3 (foundation), then V4 → V5 → V6 → V7 on Windows, then V8 and V9, then V10.

## 9. Risks

| Risk | Mitigation |
| --- | --- |
| Moving approvals and send logic into the engine breaks the chat view | Do V1 behind the existing tests, then add tests for steer/queue/resume parity before switching callers |
| Two processes disagree about state | The engine is the only writer; the island renders snapshots plus diffs with sequence numbers and resyncs on any gap |
| Windows composition backdrop won't co-exist with GPUI's DComp target | It's a spike with the v1 capture blur as the shipped fallback |
| A GPUI process can't get under 18 MB | Measure in V0/V2; if not, the hidden island runs without any GPUI window (the Win32/AppKit watcher only) and creates the window on hover |
| macOS MediaRemote keeps closing | Adapter plus AppleScript fallback; music degrades to "open the player" |
| GNOME has no layer-shell or blur | Solid material, under the top bar; a GNOME Shell extension is a later option |
| An island that interrupts is worse than none | Banners only for needs-you by default; Focus mode; per-activity toggles |

## 10. Decisions (made 2026-10-27)

1. **Process split:** `dove-island` ships as a separate binary.
2. **Dove's window closes with nothing running:** the engine exits; only the island stays.
3. **Start at login:** the island is on by default (Settings turns it off).
4. **Glass default:** dark.
5. **Order after the foundation:** look and feel on Windows first (V5, V6, then V7), then the macOS and Linux ports (V8, V9). **V4 changes:** the design is done directly in code, not drawn in Paper first; §4–§5 are the spec, and each state is reviewed from screenshots of the running build.
