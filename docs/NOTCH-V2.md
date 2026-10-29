# Notch v2: the best Dynamic Island on macOS, Windows and Linux

Status: **V0–V7 and V10 done on Windows** (2026-10-27); **V8 (macOS) and V9 (Linux) not started**: they need those machines to build and test on. Decisions in §10. **V11 (calm revamp, §11) done on Windows** (2026-10-27); see its status below.

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
- **Type.** Inter and JetBrains Mono (already bundled) with tabular numbers for timers, progress and counts. Six sizes only: 11, 12, 13, 15, 20, 40.
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
- Two lines: the title in 13 semibold; then what it wants, in 12, with the command or file in JetBrains Mono.
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
  - The title in 13 medium, and under it a live ticker of the current step in JetBrains Mono 11 that slides up as it changes.
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
- A large ring (120 px) with the time centered in Inter 40 tabular figures. The ring drains smoothly, and the last 10 s pulse.
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
| **V11: Calm revamp** | §11: snappier motion, a rebuilt agent view, zeron's Solar icons across Dove, 8 notch tabs to 5, one primary action per screen, secondary actions on hover, in one `…` menu, on keys and in the command bar | L | Motion targets met; agent view done-when met; every feature still reachable; no screen with more than one filled control |
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

## 11. V11: Calm revamp (plan, 2026-10-27)

**Status (2026-10-27): built on Windows.**
- **Icons:** all of Dove now uses Solar Linear. 121 icons were copied from zeron, and 15 of Dove's own plus 14 new ones came from the Solar set; brand marks are unchanged. `THIRD_PARTY_NOTICES.md` updated; the Hugeicons license file removed.
- **Structure:** the tab bar is Home, Agents, Music, Tray and Notifications. Notes, Timer and Dev servers are pages with a back arrow. A menu inside the island (`views/menus.rs`) holds the secondary actions, opened by right-click or `…`. Allow is a split button whose chevron holds Always and Allow all. Tooltips name every icon-only control.
- **Agent view:**
  - the engine keeps the last turns per task and streams a watched task's text at most every 100 ms (`dove-core` attention, with tests);
  - replies render as Markdown (pulldown-cmark: inline runs, lists, code blocks with Copy, links);
  - your messages show as bubbles with sending or queued state, and it stays scrolled to the newest text with a "Jump to latest" pill;
  - approvals and questions sit at the bottom one at a time, and questions are answered in place (`Engine::answer`);
  - the composer grows to 6 lines, with `@` attachments, hold-to-talk and quick replies;
  - Stop has an Undo.
- **Motion:**
  - opening is 90 ms intent with an instant open when moving up fast, a window warmed by an approaching pointer and kept for 60 s, and the dot overlapped with the spring;
  - springs are tuned by direction, content reveals at 60 %, rows rise in a shorter stagger, pages cross-fade, and digits roll;
  - album art and the timer ring fly from the slim island;
  - `DOVE_NOTCH_TIMING` and `DOVE_NOTCH_CYCLE` measure it, and `notch_sheet.ps1 -Motion` prints the timings.
- **Ideas 1–12 (§11.9):**
  - Ctrl+Alt+A / D, registered only while an approval shows;
  - Allow all for a burst of the same tool;
  - drops onto an agent row or the island;
  - copied-error popup;
  - PR checks (Home card, slim activity, popup, Merge);
  - dev-server crash popup;
  - push-to-talk via the engine's dictation;
  - Ask Dove (`?` or `ask …`);
  - usage popup at 80 %, dismissed per limit window;
  - automations card with Run now;
  - approvals held during a focus timer;
  - screenshot markup popup (arrow, box, highlight; send or keep).
- **Measured (debug build, this laptop):**
  - warm open: first frame in 3–5 ms; settled in 370–400 ms;
  - close: 340–390 ms;
  - worst frame gap: ~80 ms, from debug-build render cost;
  - the 260 ms open and 180 ms close targets are not met in debug; a release build has not been measured;
  - idle island: 2–4 MB working set, ~34 MB private, 0–0.5 % CPU.
- **Not done or limits:**
  - pages fade in, but the old page doesn't fade out, and there is no 2 % scale (GPUI divs have no transform);
  - no agent-mark flight from row to view;
  - clicking a file step opens the thread in Dove (the island has no diffs);
  - `@` attachments are sent as `@path` mentions;
  - the crash popup has no log lines (the island can't read another process's output);
  - the screenshot is the window behind, not a region;
  - push-to-talk needs local voice installed;
  - PRs need the GitHub CLI;
  - push-to-talk, merge, PR checks and Ask Dove were built and compile, but weren't exercised end to end here (no live agents, voice model or PRs on the QA home);
  - the main app was checked at its sidebar and home only after the icon swap.

**Why.** The first pass put every feature on the surface. The code has 57 buttons (25 icon buttons, 22 pills, 10 white primaries), and the expanded header alone carries 11 controls: 8 tabs, search, `+` and Pin. V11 keeps **every feature** but makes the island quiet: fewer places to go, one obvious action per screen, and everything else on hover, in one menu, on the keyboard, or in the command bar.

### 11.1 Rules

1. **One primary action per screen.** It's the only filled (white) control. Nothing else is filled.
2. **Show at rest only what you'd use most of the time.** Secondary actions appear on hover. Rare ones go in a single `…` menu per item, which right-click also opens. Every action stays reachable from the command bar and the keyboard.
3. **Fewer destinations.** 5 tabs instead of 8; the other screens open from where they're relevant.
4. **No borders where spacing will do.** Cards lose their hairlines and sit on a 4 % wash. Dividers become 12 px gaps. The amber needs-you hairline stays, because it carries meaning.
5. **One icon language, app-wide:** zeron's **Solar Icons, Linear** (1.5 stroke, 24 grid; CC BY 4.0, 480 Design), at 16 px in rows and 18 px in the header. Agent brand marks keep their colours. Text labels replace icons wherever a word is clearer.
6. **No hint text that a tooltip can carry.** "A allow · D deny" and "Ctrl+Enter saves" move into tooltips. Keys still work.

### 11.2 Icons (all of Dove, decision §11.6.1)

Dove has 150 icons (134 constants in use): 23 brand and agent marks, 121 with the same name as a zeron icon but drawn in Hugeicons, and 16 of Dove's own.
- **The 121 shared names:** replace each file with zeron's (`zeron/crates/ui/assets/icons`). Names and constants stay the same, so call sites don't change.
- **Dove's 16 own** (`clipboard-list`, `dove-logo`, `media-next/pause/play/previous/shuffle`, `music-note`, `note-text`, `phone`, `send-up`, `sparkle-agent`, `split-rows`, `timer`, `users-group`, `widget-add`): take the matching Solar Linear glyph. The hand-drawn ones go. `dove-logo` stays (it's the brand).
- **Brand and agent marks** (23) keep their own art and colours.
- **Sizes:** Solar is drawn for a 1.5 stroke at 24. Check the app at its common icon sizes (14, 16, 18) for glyphs that read heavier or lighter than before, and adjust sizes at call sites, not the files.
- **Credit:** "Solar Icons by 480 Design (CC BY 4.0)" in `THIRD_PARTY_NOTICES.md` and in the comment on Dove's icon list.
- **Review:** the app's own screenshots (sidebar, composer, settings, Flock, call view) as well as the notch's contact sheet, since this changes every screen.

### 11.3 Structure: 8 tabs → 5 (decision §11.6.2)

| Today (tab) | V11 | Still reachable by |
| --- | --- | --- |
| Home | **Home** (tab) | |
| Agents | **Agents** (tab) | |
| Music | **Music** (tab) | |
| Tray | **Tray** (tab) | |
| Notifications | **Notifications** (tab); its unread dot stays on the tab | |
| Notes | A page, no tab | A Home card (the pinned note, or the latest), with "All notes"; the slim pinned note; the command bar (`note …` creates one, `notes` opens the list) |
| Timer | A page, no tab | A Home card while a timer runs, and a "Start a timer" suggestion when none does; the slim ring; the command bar (`timer 25`) |
| Dev servers | A page, no tab | A Home card while a server is listening; the command bar (`ports`, `localhost`) |

The pages without tabs open with a small back arrow in the header's place of the puck, and Esc or Backspace returns to where you came from. Settings › Tabs keeps working on the 5 tabs; hidden ones stay reachable from the command bar.

### 11.4 Screen by screen

| Screen | Today at rest | V11 at rest | Moved, not removed |
| --- | --- | --- | --- |
| **Header** | 8 tabs, search field, `+`, Pin (11) | 5 icon tabs with the puck, and a search icon that grows into the field (6). Typing anywhere starts a search | `+` → "New agent" at the top of Agents, plus `new …` in the command bar. Pin → the header's `…` menu (right-click) and Ctrl+P. The island already stays open while a field has focus |
| **Banner** | Dismiss, Deny, Always, Allow (+ key hint) | **Deny** and **Allow**; Allow is a split button whose chevron offers Always | Dismiss → Esc or a swipe up. Key hint → tooltip |
| **Slim** | As is | As is, without the "+N" chip (the stacked marks and ring say enough) | The count moves into the hover tooltip |
| **Home** | Up to 6 cards, each with a header row and count | 3 cards at most, in attention order (needs you, working, now playing, timer, dev servers, note, clips), without header rows | Cards that don't fit show as a one-line "2 more" row |
| **Agents list** | Rows, plus 3 hover icons (message, stop, open) | Rows; hover shows one `…` | `…`: Message, Stop, Open in Dove. Click still opens the agent view. The rate strip becomes one line under the list that appears only above 50 % |
| **Agent view** | Back, Stop, Open, branch and worktree chips, 3 quick replies, @ chip, Send (+ approvals) | Title row (Stop only while it runs), the reply, the composer with Send | Back → the Agents tab, Esc or Backspace. Open → click the title. Branch and worktree → one faint mono line. Quick replies → suggestions shown only while the composer is empty and focused. @ → type `@` for a shelf picker |
| **New agent** | Prompt, agent chips, project chips, worktree pill, Start | Prompt and **Start**; one line "Claude Code · dove · worktree off" | Click the line to change the agent, project or worktree (an inline picker) |
| **Music** | Shuffle, prev, play, next, device chip, scrubber | Prev, **play**, next, scrubber | Shuffle and the device appear on hover, top right. Volume stays on scroll |
| **Tray** | 6 filter chips, capture, clear, tiles with 3 hover icons | One filter word ("All ▾") and the tiles | Filter menu: the 6 filters, Capture screen, Clear unpinned. Tile hover: one `…` (Send to agent, Pin, Copy path, Remove). Click copies, double-click pastes back, drag out |
| **Notes** (page) | New, list, editor, Edit, Save, Cancel, Pin, Delete, Send as prompt | List with "New", and an always-editable note that saves itself | Pin, Delete, Send as prompt → the note's `…`. Edit mode and Save go away (autosave after 600 ms and on close) |
| **Timer** (page) | Ring, 7 presets, Start, Reset, Link | Ring, 4 presets (5 · 15 · 25 · 45), **Start** | Reset shows only while a timer exists. Link to an agent → `…`. Other times by scrolling the ring (exists) or `timer 12` |
| **Notifications** (tab) | Clear all, rows | Rows; Clear all in `…` | |
| **Dev servers** (page) | 3 icons per row | Click a row to open it; hover shows `…` | `…`: Copy URL, Stop |

Result: the header goes from 11 controls to 6, the agent view from about 11 to 2–4, Tray from 8 (plus 3 per tile) to 1 plus a hover `…`, and Timer from 10 to 5. Nothing is deleted.

### 11.5 New pieces this needs

- **An in-island menu** (`notch/menu.rs`): a small glass sheet anchored to its trigger, keyboard-navigable, closing on Esc or a click outside. The window region clips to the island's shapes, so the menu must draw **inside** the island; the island springs taller to fit it when needed.
- **A split button** for Allow ▾ Always.
- **A `@` picker** in the composer, and the agent/project/worktree inline picker for New agent: both built on the menu.
- **Autosave for notes** (debounced), with the existing Undo for delete.
- **Tooltips** for icon-only controls (name plus shortcut), shown after 600 ms.
- **Command-bar verbs** for everything that left the surface: `pin`, `stop <agent>`, `open <agent>`, `ports`, `capture`, `clear clips`, `shuffle`, `link timer`.
- **Tokens:** drop `HAIRLINE` from cards; add `WASH_CARD` (4 %); icon sizes `ICON_ROW` 16 and `ICON_HEADER` 18; one control height (28) and one pill radius.

### 11.6 Decisions (made 2026-10-27)

1. **Icons:** all of Dove moves to zeron's Solar Linear, not just the notch (§11.2).
2. **Tabs:** Home, Agents, Music, Tray and Notifications. Notes, Timer and Dev servers become pages reached from Home, the slim island and the command bar (§11.3).
3. **Pin:** leaves the header for the header's `…` menu and Ctrl+P.

### 11.7 Motion: snappy first, then smooth

**Why it feels slow today.** From pointer rest to a settled island takes about 650 ms, in sequence:
- a 160 ms hover delay;
- a 160 ms dot drop, which must finish before the island starts to grow;
- a spring (stiffness 420, damping 0.68) that takes about 300 ms to settle;
- then rows rising for 220 ms plus 18 ms per row (up to 216 ms more).

On top of that, after 10 s hidden the window is destroyed, so the next open also pays for creating a window.

**Targets.**

| Moment | Today | V11 |
| --- | --- | --- |
| Pointer rest → first frame moves | ~160 ms + window creation | ≤ 100 ms, and ≤ 16 ms after the intent fires |
| Open → settled | ~650 ms | ≤ 260 ms |
| Close → gone | ~400 ms | ≤ 180 ms |
| Tab switch | 220 ms puck + rises | ≤ 160 ms, no layout jump |
| Click → visible response | next spring step | same frame (press state) |
| Dropped frames while moving | not measured | 0 at 60 Hz on the Intel UHD laptop |

**Changes.**
1. **Intent, not just a delay.** The hover delay becomes 90 ms. When the pointer is moving up toward the hot zone fast, it opens at once, because it's clearly headed there. A pointer sliding along the top edge still waits, so crossing tabs in a browser doesn't open it.
2. **The window is warm before you arrive.** When the pointer comes within 48 px of the hot zone, or the hotkey is pressed, the window is created (hidden). It's kept for 60 s after the last use instead of 10 s. Memory is still trimmed after that, so the idle budget holds.
3. **The dot and the spring overlap.** The island springs from the dot's position as it drops (no wait), so the dot reads as the start of the motion, not a step before it.
4. **Two springs, tuned by direction.** Open: stiffness 700, damping 0.78 (one small overshoot, ~220 ms). Close: critically damped, stiffness 900 (~150 ms, no bounce). Changes of size while open (tab switch, banner growth): stiffness 600, critically damped. Every spring keeps its velocity when retargeted (exists).
5. **Content leads, not lags.** Content fades in over 90 ms once the shape is 60 % open, instead of waiting for the end. Rows rise 4 px over 140 ms, staggered 12 ms, for the first 4 rows only; the rest appear with the 4th.
6. **Tabs cross-fade and the height springs.** The old page fades out over 70 ms while the new one fades in, with a 2 % scale, and the island's height springs to the new page's height at the same time. The puck slides in 160 ms with the close spring.
7. **Instant feedback.** Every pressable control shows its pressed state in the same frame. A press squishes the island 2 % over 60 ms. Toggles change optimistically and roll back only if the engine refuses.
8. **Shared elements (real ones this time).** The agent mark in the slim island flies to its row when the island opens; a row's mark flies to the agent view's title row; album art grows from the slim 22 px to the player's 88 px; the timer ring grows into the big ring. Done with one "hero" layer that draws the element between its start and end bounds while the spring runs.
9. **Numbers roll.** Counts, timers and elapsed times roll digit by digit (tabular figures) instead of jumping.
10. **No layout jumps.** Pages report their height before they draw (exists), and text never reflows while the shape springs (exists). Lists keep their scroll position across updates.
11. **Measure it.** A `DOVE_NOTCH_TIMING=1` trace logs intent → first frame, open → settled and close → gone, plus frame times while moving. `notch_sheet.ps1` gets a `-Motion` mode that records those for each transition. Reduced motion keeps cross-fades only.

### 11.8 Agent view: rebuilt

**What's wrong today.**
- **The reply jumps.** Text deltas don't notify readers (by design, to save wake-ups), so the island only sees new text at the next tool call or the end of the turn.
- **It's only the last 1,200 characters,** cut mid-sentence, with no sign of what you said. After you send, all you get is a "Sent" toast.
- **The Markdown is crude.** Bold and code spans split a line into separate boxes, so a sentence with **bold** in it breaks across lines oddly. Code blocks show their backticks. Links aren't links.
- **Two placeholders overlap.** The field shows "Message this agent…" and a second hint ("Steer: it reads this now") sits beside it.
- **It doesn't follow the stream.** The reply doesn't stay scrolled to the bottom as it grows.
- **A fixed 330 px height,** whether the agent said one line or a page.
- **Approvals push the conversation away.** Several stack above the reply and can hide it.
- **Questions can't be answered here.** Only "Answer in Dove".
- **The composer is one line,** so longer messages are awkward.
- **Stop has no Undo,** and there's no sign of messages queued for the end of the turn.

**The new agent view.**
- **Title row.** The mark in its progress ring, the title (click to open in Dove), one status word in its colour, and Stop while it runs. A faint mono line under it: `feat/login · worktree · 4m · $0.12`.
- **A real mini transcript,** newest at the bottom, sticking to the bottom while it streams (unless you scroll up, then a "Jump to latest" pill):
  - **your messages** as right-aligned bubbles, including queued ones (marked "queued" until the agent reads them);
  - **the agent's replies** with proper Markdown: headings, bold and italic inline (one text run, no line breaks), code spans, fenced code blocks in JetBrains Mono with a Copy button on hover, lists, links (open in the browser);
  - **tool steps** folded into one line per turn ("Edited 3 files · ran tests ✓ · 1 failed"); click it to unfold the list of steps, and click a file step to see its diff;
  - the last 3 turns, with "Earlier in Dove" at the top to open the full thread.
- **Streaming, at low cost.** The engine announces text at most every 100 ms per task, and only to readers that asked for it (the island while that agent's view is open). Every other reader still gets the existing per-event pings, so idle cost doesn't change.
- **Approvals pinned at the bottom,** above the composer, one at a time, with a diff preview and "1 of 3" when there are more. The conversation stays visible above. Deny keeps its Undo.
- **Questions answered in place:** the question with its options as buttons (and free text when it allows it), sent with `Engine::answer`.
- **The composer:**
  - grows from 1 to 6 lines; Enter sends and Shift+Enter adds a line;
  - its placeholder says what will happen: "Steer…" or "Queue for after this turn…";
  - `@` opens the tray picker to attach files and clips (sent as attachments, not pasted paths);
  - the mic, for push-to-talk through Voice Mode when it's installed;
  - quick replies ("Continue", "Run the tests", "Explain") show only while the composer is empty and focused.
- **Height follows the content.** At least 220 px, at most the island's maximum, springing as the reply grows.
- **Stop has an Undo** for 3 s, like Deny.
- **Keyboard:** Esc goes back; ↑ in an empty composer edits your last queued message; A and D answer the visible approval; Ctrl+O opens the thread in Dove.
- **Done when** a streaming reply reads smoothly with no jumps, a long reply with code and lists renders like Dove's transcript, you can approve, answer, steer, queue and stop without leaving the island, and the view looks right at 1 line and at 200.

### 11.9 More things the notch does (picked 2026-10-27: 1–12 in V11; 13 and 14 later)

Agent work first, since it's what no other notch can do.

| # | Idea | What it does | Size |
| --- | --- | --- | --- |
| 1 | **Approve from anywhere** | Global keys (Ctrl+Alt+A / D) answer the banner's approval without moving the pointer | S |
| 2 | **Batch approvals** | "Allow all 4 reads in `src/`": one decision for a burst of similar safe requests | S |
| 3 | **Drop to delegate** | Drag files, a screenshot or selected text onto the island or an agent row: start a new agent with them, or attach them to that agent | M |
| 4 | **Copy an error, fix it** | Copy a stack trace or failing-test output, and the tray offers "Ask an agent to fix this" | S |
| 5 | **PR and CI activity** | After an agent opens a PR: a live activity with checks running or failing, then Merge, from the island | M |
| 6 | **Dev-server watch** | When a dev server an agent started crashes, a banner with its last log lines and "Ask the agent to fix it" | M |
| 7 | **Push-to-talk to an agent** | Hold the hotkey and speak; it's transcribed into the agent's composer (local voice model) | M |
| 8 | **Ask Dove** | Type a question in the command bar and get a short answer inline from the default agent, in a scratch thread | M |
| 9 | **Usage running low** | A popup banner when a plan passes 80 % of a limit, with the meter and when it resets. It stays until you dismiss it, then stays quiet for that limit window | S |
| 10 | **Automations** | Next scheduled run, "Run now", and the last run's result | S |
| 11 | **Focus with batching** | During a focus timer, approvals queue silently and arrive together when it ends (risky ones still break through) | S |
| 12 | **Screenshot and annotate** | Capture a region; the shot pops up in the island like the usage banner, where you draw an arrow, box or highlight on it and send it to an agent (or keep it in the tray). It goes away when you dismiss or send it; no separate window | M |
| 13 | **Continue on phone** (later) | Hand the agent view to Dove's phone remote with one click | S |
| 14 | **Agent cost today** (later) | A tiny daily total of tokens and spend across agents, on Home | S |

### 11.10 Order and done-when

1. Icons app-wide (§11.2) and the notch's tokens.
2. Motion (§11.7): intent, warm window, retuned springs, content timing, the timing trace. First, because every later screen is judged by it.
3. The menu, split button and tooltips (§11.5).
4. Header and the 5-tab structure.
5. The agent view (§11.8), including throttled streaming in `dove-core`.
6. Every other screen in §11.4.
7. Shared elements and rolling numbers (§11.7 items 8 and 9).
8. Command-bar verbs.
9. Ideas 1–12 from §11.9, in this order: 1, 2, 11 (approvals); 9 (usage popup); 4, 3 (clipboard and drop); 12 (screenshot popup); 6, 5 (dev servers, PR/CI); 10 (automations); 7, 8 (voice and Ask Dove). Both popups (9 and 12) use the banner presentation and its dismiss.
10. Contact sheet (`scripts/notch_sheet.ps1`) before and after, the `-Motion` timings, and a keyboard pass.

Done when the motion targets in §11.7 are met in the timing trace, the agent view meets its own done-when (§11.8), every action in §5.2 is still reachable (each checked by pointer, keyboard and command bar), no screen has more than one filled control, and the contact sheet reads calmer at a glance. One build at the end.
