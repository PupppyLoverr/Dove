# The Dove notch

Status: **built on Windows** (2026-10-27), in `apps/dove/src/notch/`. macOS and Linux report the platform as unsupported and keep the notch off; their native layers (§4) are still to do. Part of [ROADMAP.md](ROADMAP.md) §3.5.
Reference: [Syllo](https://trysyllo.app/) (a Dynamic Island for Windows and macOS: music, a file shelf, clipboard history, and coding-agent status with approvals).

## 0. What shipped, and where it differs from the plan below

| Area | Built |
| --- | --- |
| Window | A fixed 500 × 420 transparent canvas at the top center (`mod.rs`). Each frame its window region is clipped to the island (`platform::Handle::shape`), so clicks around it reach the app below. Hidden = not shown. |
| Motion | Opening drops a 13 px dot from above the edge (170 ms), then springs it into shape (560 ms, `1 − e^(−7t)·cos(8t)`, about 6 % overshoot). Closing shrinks back to the dot and lifts it away. Size changes inside a state (a page, a new agent) glide with the same spring. Reduced motion snaps. |
| Glass | DWM acrylic ignores window regions (it blurs the whole rectangle), so the island makes its own frosted glass: the screen behind the canvas is captured with GDI (the notch excludes itself with `WDA_EXCLUDEFROMCAPTURE`), shrunk to 1/8, blurred and drawn under a dark tint, refreshed every 0.4 s open and 2 s slim, with no repaint when nothing behind it changed. |
| Slim | A 34 px capsule: each agent kind with work going on (chat threads and Flock panes) with its count and an orange dot when it needs you; the song (art and an equalizer); the timer. A third activity gets a round bubble beside it. Shown while something is live, or always (setting). Hover or click opens it. |
| Alerts | Approvals pop up with Deny / Always / Allow, or Review in Dove for anything the safety classifier flags (deletions, force pushes, work outside the folder) and for questions. Finished and failed tasks flash for 5 s; a click opens the agent. |
| Expanded | Tabs: Home, Agents, Music, Clipboard, Notes, Timer, plus `+` (new agent) and Pin. The agent view shows its status, steps, latest reply and approvals, and has a message field (send = the composer's Send: steer, queue or resume), Stop and Open in Dove. The new-agent form picks the agent, project and worktree, then starts it the way the home composer does. |
| Music | Windows media session (GSMTC) on its own thread: art, title, artist, app, position, play/pause/next/previous. |
| Clipboard | History while Dove runs (text and images, concealed items skipped, pinned text saved); click to copy back, send text to an agent. The shelf takes files dropped on the island and drags them out (OLE, the shell's own data object), copies paths and sends them to an agent. |
| Notes, Timer | Saved notes with edit and delete; a countdown with presets, ±1 min, pause and an OS notification at zero. |
| Settings | Notifications › Notch: Show the notch, Keep the slim island visible, Live activities, Clipboard history. |
| Not yet | macOS and Linux; messaging a Flock pane from the notch (it opens Flock); multi-monitor (primary display only). |

## 1. What it is

A small black island at the top center of the screen that belongs to Dove.

- **macOS:** it wraps the MacBook's camera notch, so it looks like the notch itself grew. On Macs without a notch (and external displays) it draws the same pill just under the menu bar.
- **Windows and Linux:** nothing is visible until the pointer reaches the top middle of the screen. Then the island drops down. It goes back up when the pointer leaves.
- **Live activity:** without any hover, it briefly widens on its own when something needs you: an agent asks for permission, a task finishes or fails, a track changes. Then it shrinks back.

It shows what Dove's agents are doing (chat threads and Flock panes), what's playing, and a **shelf**: drag anything onto it to keep it, and drag it back out later to attach it to a thread or drop it into any app.

Why Dove can do this better than Syllo: Syllo watches agents from the outside through hooks. Dove *is* the runtime. The notch reads the same `Engine` state as the sidebar, so it can approve, answer, steer, stop, start and hand off with the same calls a click makes, for every agent Dove supports.

## 2. States

```text
 hidden ──hover top-center (150 ms)──▶ peek ──hover/click──▶ open ──pin──▶ pinned
   ▲  ▲                                  │                      │
   │  └──────── leave (400 ms) ──────────┴──────────────────────┘
   │
   └── live activity (3–6 s, no focus taken) ◀── agent needs you / finishes / fails, track changes, shelf drop
```

| State | Size (logical px) | Content |
| --- | --- | --- |
| **Hidden** | macOS: exactly the hardware notch (draws nothing extra). Windows/Linux: off-screen, plus an invisible hot zone | — |
| **Live activity** | Width 360, height = notch/menu-bar height | Left: agent mark or album art. Right: status ("Needs you", "Done +120 −8", elapsed time) or an equalizer. Needs-you stays until handled or dismissed |
| **Peek** | 420 × 120 | The top 3 attention items (needs you → working → recently done), the current track, a shelf count |
| **Open** | 640 × 380 | Tabs: **Agents · Shelf · Music** (+ optional modules). Full keyboard control |
| **Pinned** | Open, but stays until unpinned or Esc | For watching a long run |

Motion uses the revamp-v3 motion system: a spring for the width/height morph, content cross-fades, and reduced motion becomes a plain fade. Collapsed and idle, it renders **zero frames**.

## 3. Modules

### 3.1 Agents (the core)

The source is the **attention model** the sidebar already sorts by (needs you, working, unread, failed), extended with Flock panes (their PTY state: working, idle or waiting) and teammates.

Each row shows the agent mark, the title, project · branch, status, elapsed time, the last tool line ("Editing `auth.rs`", "Running tests"), and tokens/cost for the turn.

Actions, all through the same paths the main window uses:

| Action | How |
| --- | --- |
| Approve / deny a permission request | A / D keys or buttons (the transcript's permission card, compacted). The same safety rules as Voice Mode: deletions, force pushes and work outside the thread's folder need the full window |
| Answer a question | 1–9, like the transcript |
| Quick reply / steer | A one-line composer in the row (`TaskView::submit`: steer when working, queue or send when idle) |
| Stop | `TaskView::interrupt` |
| Open | Focuses Dove on that thread, or the Flock pane |
| New task | A mini composer: project, agent, prompt, worktree on/off. Voice button for push-to-talk via Voice Mode |

Also in this tab:
- **Rate-limit and usage meters.** The harnesses already emit rate-limit windows ("Claude 5-hour window at 72 %"), and a one-time warning shows when a plan runs low.
- **Team** runs grouped under their parent, with fan-out progress ("3 of 4 children done").
- **Race groups** (ROADMAP §3.2) show as one row with N mini progress bars and a "Compare" button.

Later (N5): agents run in plain terminals *outside* Dove (Claude Code and Codex hooks, as Syllo does), shown read-only with a "Bring into Dove" action.

### 3.2 Shelf (drag clipboard)

Drop anything on the island (or press the hotkey while it's open and paste) and it's kept:

- **What it holds:** files and folders (copied or referenced, user's choice; default referenced up to 50 MB, copied for temp/screenshot sources), images, text snippets, URLs, and screenshots taken from the notch.
- **Storage:** `~/.dove/shelf/` for blobs and a `shelf_items` table in `dove-store` (id, kind, name, path or blob, size, source app, created, pinned, tags). Unpinned items expire after 7 days (configurable).
- **Taking things out:**
  - **Drag out** to any app (Explorer/Finder, Slack, a browser upload field) or onto Dove's composer, which already accepts file drops.
  - **Attach to…** sends it to a thread's composer (pick from the attention list), or straight to a running agent as a steer with the file attached.
  - **Copy** puts it on the system clipboard.
- **Multi-select:** drag several items out at once, or "attach all".
- **Clipboard history** (N4, opt-in): text and images you copy appear as a "Recent" lane above the shelf. It never records content marked concealed: Windows' `ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory = 0`, macOS `org.nspasteboard.ConcealedType` / `TransientType` (what password managers set). Pause it with one click.
- **Agents can read the shelf** (opt-in): MCP tools `shelf_list`, `shelf_get` and `shelf_put` on the existing `dove` server, so "use the screenshot I shelved" works in every harness.

### 3.3 Now Playing

Artwork, title, artist, a progress bar, play/pause/next/previous and seek. In live activity: art on the left and a small equalizer on the right while playing.

| OS | Source | Notes |
| --- | --- | --- |
| Windows | `GlobalSystemMediaTransportControlsSessionManager` (WinRT, `windows` crate `Media_Control`) | Event-driven: session, playback and properties changed. Covers Spotify, browsers, Apple Music, YouTube Music |
| macOS | MediaRemote | Since macOS 15.4, third-party apps can't read it directly. Use the known adapter approach (read it through `/usr/bin/perl`, which is still entitled) and fall back to Spotify/Music AppleScript. Needs a spike |
| Linux | MPRIS over D-Bus (`zbus`) | `PropertiesChanged` signals, no polling |

Artwork is decoded once per track and kept as one GPUI image, freed on track change.

### 3.4 Extras that make it worth keeping on

Ordered by value. Each is a module the user can turn off.

1. **Voice Mode in the island.** While a call is on, the island shows the orb, the live waveform and mute. Hold the island to talk. (Voice Mode exists; this is a second view of it.)
2. **Notification center.** Completions, failures, PR checks passing or failing, PR merged, automation runs. One list, cleared as you handle them. It replaces OS toasts while the notch is on (setting).
3. **Dev servers and ports.** Running preview servers per worktree (`localhost:5173 · login-page`), with open, copy URL and stop.
4. **Screenshot to agent.** Capture a region or window, then drop it on the shelf or send it straight to a thread. (Voice already captures windows on Windows.)
5. **Focus / Do not disturb.** Only needs-you items break through; the rest is batched until focus ends.
6. **Phone handoff.** Shows when the phone remote is connected and which device approved what.
7. **Quick launcher.** Type in the open island: thread search, actions and projects, the same results as the command palette (Ctrl K).

Left out on purpose (Syllo has them, but they aren't Dove's job): calendar, Pomodoro timers, notes, window snapping, file conversion. They're cheap to add later as modules if users ask.

## 4. Platform mechanics

The notch is a **second GPUI window** in the same process: `WindowKind::PopUp`, transparent background, no titlebar, `focus: false`, not resizable or movable. It lives as long as the process (which the tray's keep-running mode already allows with the main window closed).

| Concern | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Find the spot | `NSScreen.safeAreaInsets.top` and `auxiliaryTopLeftArea`/`auxiliaryTopRightArea` give the notch's exact width and height; without a notch, the menu-bar height | Top-center of the chosen monitor's work area, per-monitor DPI | Top-center; on GNOME, under the top bar (it owns the top middle with its clock) |
| Stay on top | Window level above the menu bar (`NSMainMenuWindowLevel + 1`); `canJoinAllSpaces`, `stationary`, `fullScreenAuxiliary`, `ignoresCycle` | `WS_EX_TOPMOST`, `WS_EX_TOOLWINDOW` (no taskbar button or Alt-Tab), `WS_EX_NOACTIVATE` | Wayland: layer-shell, `Overlay` layer, top anchor, exclusive zone 0 (zui already has `WindowKind::LayerShell`). X11: `_NET_WM_WINDOW_TYPE_DOCK` + above. GNOME Wayland has no layer-shell: an always-on-top normal window under XWayland, or the GNOME extension route later |
| Reveal on hover | The hardware notch area is the hot zone: tracking area on the window | A 1–2 px tall invisible topmost strip at top-center catches mouse-enter (no polling) | Layer-shell: a thin input-only surface; X11: same strip |
| Click-through when collapsed | `ignoresMouseEvents` toggled by state | Toggle `WS_EX_TRANSPARENT` / `WS_EX_LAYERED` | Wayland: shrink the input region; X11: XShape input region |
| Hide when | A full-screen app is in front (setting) | `SHQueryUserNotificationState` says full-screen / presentation / D3D full-screen | `_NET_WM_STATE_FULLSCREEN` on the active window |
| Drop in | GPUI `FileDropEvent` handles file paths. Text, URLs and images need a small native drop target: `NSDraggingDestination` | Register an `IDropTarget` for `CF_UNICODETEXT`, `CF_DIB`, URLs | XDND / `wl_data_device` mime types |
| Drag out | `NSDraggingSession` with file URLs (`NSPasteboardItem`) | `DoDragDrop` with an `IDataObject` carrying `CF_HDROP` (+ text) | `wl_data_source` / XDND source. Hardest; ship last |
| Global hotkey to open | Carbon `RegisterEventHotKey` (or `CGEventTap`) | `RegisterHotKey`, already in `hotkey.rs` | Portal `GlobalShortcuts` on Wayland; `XGrabKey` on X11 |

**What GPUI doesn't give us (needs platform code or a small zui patch):** click-through toggling, window level and collection behavior on macOS, extended Windows styles, drag *out* to other apps, and non-file drops. `raw-window-handle` is already a dependency, so most of this is raw-handle code in `notch/platform/*`. Anything that would be cleaner inside zui goes to `vendor/` with a note in `vendor/README.md`, the way `gpui-base` is patched today.

**Known conflicts to handle:**
- **Windows 11 Snap Layouts** appear when a window is dragged to the top center. The hot zone ignores entry while a mouse button is held (a window drag is in progress).
- **Several monitors:** the notch is on the monitor with the pointer, the primary one, or all (setting; default: the primary, plus whichever has the pointer).
- **Auto-hide taskbar or menu bar** at the top: offset below it.

## 5. Code layout

```text
apps/dove/src/notch/
  mod.rs         NotchWindow entity: state machine, layout per state, open/close, settings
  model.rs       NotchModel global: the attention list (shared with sidebar, tray, notifications, phone, voice)
  agents.rs      Agents tab and live-activity rows; actions call TaskView / Flock as clicks do
  shelf.rs       Shelf tab; store access; attach-to-thread routing
  media/         mod.rs (trait MediaSource) + gsmtc.rs, mpris.rs, macos.rs
  dnd/           native drop target and drag source per OS
  platform/      mac.rs, windows.rs, linux.rs: placement, level, click-through, hot zone, full-screen detection
crates/dove-store   shelf_items table + migration
apps/dove/src/mcp_browser.rs   shelf_* tools (opt-in)
apps/dove/src/settings.rs      Settings › Notch
```

The attention model moves out of `sidebar.rs` into `notch/model.rs` (or a neutral `attention.rs`) so the sidebar and the notch can't disagree. The notch subscribes to it; it never polls the engine.

## 6. Settings › Notch

On/off (default on for macOS with a notch, on for Windows; ask at first run on Linux) · display (primary / pointer / all) · hover delay (default 150 ms) · live activity: which events, and for how long · modules on/off and order · hide in full screen · shelf: copy vs reference, size limit, expiry · clipboard history on/off (default off), with an excluded-apps list · agents may read the shelf (default off) · hotkey.

## 7. Performance budget

| Measure | Budget |
| --- | --- |
| Extra RSS with the notch on, collapsed | ≤ 15 MB (measure the second window's swapchain in the N0 spike first) |
| Idle renders while collapsed | 0 |
| Idle CPU while collapsed | ≤ 0.1 % of a core: hot zone and media are event-driven, no polling |
| Hover → peek visible | ≤ 100 ms after the delay |
| Morph animation | Display refresh rate, no dropped frames on a 2020 laptop |
| Music, while playing and collapsed | Equalizer only in live activity, ≤ 30 fps, stops when hidden |

These go into `scripts/perf_check.ps1` and the CI perf gate.

## 8. Phases

| Phase | Contents | Size | Done when |
| --- | --- | --- | --- |
| **N0 spike** | The window on all three OSes: placement, level, click-through, hover reveal, full-screen hiding, a morph animation. Measure RSS and CPU | M | A blank island behaves right on a notched Mac, Windows 11 multi-monitor, KDE Wayland, GNOME and X11 |
| **N1 agents** | Attention model extraction, Agents tab, live activity, approve/deny/answer/stop/reply, open in Dove, Flock panes | M | You can run a day of agents without looking at the main window |
| **N2 shelf (in)** | Drops of files, text, URLs and images; store; Attach to…; copy; drag onto Dove's composer | M | A screenshot dropped on the notch ends up in a thread in two clicks |
| **N3 music** | Windows GSMTC and Linux MPRIS; macOS adapter spike, then the implementation | M | Play/pause/skip and art for Spotify and browsers on all three |
| **N4 out** | Native drag-out (Windows, macOS, then Linux), clipboard history (opt-in), shelf MCP tools | L | Drag a shelved file into Slack or a browser upload |
| **N5 extras** | Voice in the island, notification center, ports, screenshot to agent, focus mode, launcher, outside-Dove agent hooks | L | Chosen per user feedback |

## 9. Risks

| Risk | Mitigation |
| --- | --- |
| macOS MediaRemote locked down (15.4+) | Adapter approach plus an AppleScript fallback; Music can ship last on macOS |
| GNOME Wayland has no layer-shell or global positioning | XWayland fallback; document it; consider a GNOME Shell extension later |
| A second GPUI window costs too much memory | Measured in N0; if over budget, create the window lazily on first hover and drop its renderer while hidden |
| Drag-out on Linux | Ship "Copy" and "Attach to…" first; drag-out on Linux last |
| A notch that interrupts is worse than none | Live activity only for needs-you by default; everything else opt-in; Focus mode |
| Clipboard history leaks secrets | Off by default, concealed types never stored, excluded apps, one-click pause, local only |

## 10. Decisions needed

1. **Default on Windows:** on at install, or offered in the first-run wizard?
2. **Shelf default:** reference the original files, or copy them (safer if the original moves, uses disk)?
3. **Music on macOS:** accept the adapter workaround, or ship macOS without Now Playing at first?
4. **Outside-Dove agents** (hooks into Claude Code and Codex in other terminals): wanted, or Dove-only?
