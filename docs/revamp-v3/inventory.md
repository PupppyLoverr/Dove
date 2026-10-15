# Dove UI inventory: baseline for the "nothing removed" checklist

Snapshot of the working tree on 2026-10-15 (HEAD `2cd238b` plus uncommitted changes). Paths are relative to `apps/dove/src` unless shown otherwise. Line numbers are approximate; the tree is changing.

## 1. Settings

### 1a. Settings page controls (44)

| # | Section | Row label | Control | Key | Default |
|---|---|---|---|---|---|
| 1 | General | Reopen thread tabs on launch | switch | `restore_tabs` | on |
| 2 | General | Prewarm agent harness while typing | switch | `prewarm` | on |
| 3 | General | Isolate each new thread in a git worktree | switch | `use_worktree` | on |
| 4 | General | Branch prefix | text | `branch_prefix` | `dove/` |
| 5 | General | Default clone directory | text + Browse… | `clone_dir` | `~/Dove` |
| 6 | Appearance | Color theme | segmented System / Dark / Light | `appearance` | system |
| 7 | Appearance | Accent color | 5 swatches (Sky, Indigo, Orange, Green, Pink) | `accent` | sky |
| 8 | Appearance | Window surface material | segmented Glass / Acrylic / Normal | `surface` (legacy `frosted`) | glass |
| 9 | Appearance | New-thread background image | badge + Choose / Change / Remove | `wallpaper_path` | none |
| 10 | Appearance | Backdrop shader effect | segmented Original / Dither / Halftone / Scanlines | `wallpaper_effect` | dither |
| 11 | Appearance | Animate backdrop | switch | `wallpaper_animate` | off |
| 12 | Agents | per-harness Enabled / Hidden | switch, or "Not installed" | `agent_disabled.<id>` | enabled |
| 13 | Agents | Sign-in & custom paths → | link to Agents page | — | — |
| 14 | Agents | Configure Harnesses (hero) | button to Agents page | — | — |
| 15 | Crew & Team | Persistent specialist notes | switch | `team_notes` | on |
| 16 | Crew & Team | Open Crew Page | button | — | — |
| 17 | Canvas | Open designs automatically as they arrive | switch | `canvas_auto_open` | on |
| 18 | Canvas | Interactive 2-way agent bridge | switch | `canvas_bridge` | on |
| 19 | Canvas | Default artboard viewport | segmented Desktop / Tablet / Mobile | `canvas_default_device` | desktop |
| 20 | Composer | Default permission level | segmented Ask every time / Auto-edit / Plan | `permission_mode` | default |
| 21 | Composer | Projects authorized for Bypass | list + Revoke all | `bypass_ok.<project>` | none |
| 22 | Voice | Local Parakeet dictation model | Download → progress + Cancel → Installed + Remove → error + Try again | (files in `voice/`) | missing |
| 23 | Voice | Microphone | list (System default + devices) | `voice_device` | system default |
| 24 | Keyboard | one row per shortcut | badge + Rebind + Reset; "Modified" badge | `key.<id>` | per shortcut |
| 25 | Keyboard | Reset All Shortcuts (hero) | button / "Default bindings" badge | `key.*` | — |
| 26 | Notifications | System desktop notifications | switch | `notifications` | on |
| 27 | Notifications | Completion sound | switch | `notification_sound` | off |
| 28 | Notifications | Keep running in background | switch (+ tray) | `keep_running` | off |
| 29 | Remote Control | Enable remote control | switch | `remote_enabled` | off |
| 30 | Remote Control | Woozlit account | status + Sign in / Sign out | — | — |
| 31 | Remote Control | Connection status | badge | — | — |
| 32 | Remote Control | Device ID, Last error | read-only rows | — | — |
| 33 | Integrations | Dove for Chrome | status + pairing code copy | — | — |
| 34 | Integrations | Composio | switch | `integrations.json › composio.enabled` | off |
| 35 | Integrations | Composio API key | masked field | `composio.api_key` | empty |
| 36 | Integrations | Composio toolkits | 10 chips + other slugs field | `composio.toolkits` | empty |
| 37 | Integrations | Paper | switch + Running / Not running | `paper.enabled` | off |
| 38 | Integrations | Computer use | switch + Check / Detect again / install guide | `computer.enabled` | off |
| 39 | Data & Storage | Workspace & transcript storage | size + Open folder | — | — |
| 40 | Data & Storage | Storage path (DOVE_HOME) | path + Copy | env | `~/.dove` |
| 41 | Data & Storage | Telemetry & privacy | "Zero telemetry" badge | — | — |
| 42 | About | Application version | badge | — | — |
| 43 | About | License | text | — | — |
| 44 | About | Source repository | Open GitHub | — | — |

### 1b. Settings written from other surfaces (17)

| Key | Set from |
|---|---|
| `sidebar_group` | Sidebar options menu › Group by project |
| `sidebar_density` | Sidebar options menu › Compact rows |
| `sidebar_archived` | Sidebar options menu › Show archived |
| `appearance` | Palette › Switch theme (light / dark) |
| `use_worktree` | Composer checkout menu › Worktree checkbox |
| `permission_mode`, `mode.<task>` | Composer mode chip |
| `bypass_ok.<project>` | Composer › Enable bypass confirmation |
| `sidebar_open`, `sidebar_width`, `panel_open`, `panel_width`, `panel_expanded` | Sidebar and panel toggles and resizing |
| `open_tabs` | Thread tab changes |
| `active_project` | Project selection |
| `renamed.<task>` | Rename |
| `draft.<task>`, `draft.home` | Composer draft autosave |
| `add_dirs.<task>` | Composer › Folders menu |
| `automation_persona.<id>` | Automations editor |
| `picker.v1` | Composer picker memory |
| `intro.seen` | Intro completion |
| `rate_limits.<harness>`, `models.v2.<harness>` | Engine caches |

### 1c. Keys with no UI (8)

`agent_browser`, `agent_office_skills`, `frosted` (legacy), `agent_path.<harness>` (Agents page), env `DOVE_HOME`, `DOVE_DATA`, `DOVE_SHOW_INTRO` / `DOVE_SKIP_INTRO`, `DOVE_BROWSER_ENDPOINT_FILE`. QA hooks: `DOVE_ROUTE`, `DOVE_PANEL`, `DOVE_SETTINGS`, `DOVE_OPEN_THREAD`, `DOVE_OPEN_CANVAS`, `DOVE_FRAME_STATS`, `DOVE_EXPAND_TOOLS`, `DOVE_WINDOW_TITLE`.

## 2. Keyboard

### 2a. Rebindable shortcuts (17, Settings › Keyboard)

| Action | Windows / Linux | macOS |
|---|---|---|
| Command palette | Ctrl+K | ⌘K |
| New thread | Ctrl+N | ⌘N |
| Toggle sidebar | Ctrl+B | ⌘B |
| Toggle side panel | Ctrl+\ | ⌘\ |
| Chat ⇄ Terminal | Ctrl+` | Ctrl+` |
| Close thread tab | Ctrl+W | ⌘W |
| Next thread tab | Ctrl+Tab | Ctrl+Tab |
| Previous thread tab | Ctrl+Shift+Tab | Ctrl+Shift+Tab |
| Rename thread | F2 | F2 |
| All threads | Ctrl+Shift+T | ⌘⇧T |
| Kanban | Ctrl+Shift+K | ⌘⇧K |
| Back | Alt+Left | ⌥← |
| Forward | Alt+Right | ⌥→ |
| Settings | Ctrl+, | ⌘, |
| Add project | Ctrl+O | ⌘O |
| Focus the composer | Ctrl+L | ⌘L |
| Quit | Ctrl+Q | ⌘Q |

### 2b. Fixed bindings (9)

- Ctrl/⌘ 1–9: go to tab.
- Mouse back and forward buttons.
- Esc: close overlays.
- Browser: Ctrl/⌘ L (address), T (new tab), W (close tab), [ (back), ] (forward), R or F5 (reload).

### 2c. Context keys (20)

- **Composer:** Enter (send, steer or queue), Esc (cancel voice, close menu, or stop), Ctrl/⌘ V (paste image).
- **@ menu:** ↑, ↓, Tab, Esc.
- **Model picker:** Esc, ↑, ↓, Enter or Space, Tab or Shift+Tab, ←, →, Home, End.
- **Palette:** ↑, ↓, Esc.
- **Transcript:**
  - Alt+↑ / Alt+↓ step between prompts; PageUp / PageDown; Ctrl/⌘ Home and End.
  - A / S / D: allow once, allow always, deny.
  - 1–9: answer a question.
- **Terminal:** Ctrl+Shift+C / V; Enter restarts an exited shell; PTY key map.
- **Browser address bar:** Enter, Esc. On Linux, Ctrl+C / X / V are forwarded to the page.
- **Settings › Keyboard recording:** Esc cancels.
- **Intro:** Esc, Enter, Space or click skips.

## 3. Modals, menus and overlays

| Overlay | Items |
|---|---|
| Sidebar "More" menu | Agents, Usage, Archived, Settings, Help & shortcuts |
| Sidebar options menu | Group by project, Compact rows, Show archived, Add project… |
| Thread context menu | Rename (F2), Pin/Unpin, Archive/Unarchive, Copy ▸ (Title, Branch name, Path, Session ID), Delete… (click again to confirm) |
| Thread info card | Local · host, project, branch, harness/model, usage, context; New side chat, Fork thread, Open folder, Open in system terminal, Copy path |
| Command palette | Actions: New thread, All threads, Team, Kanban, Automations, Pull requests, Agents, Usage, Archived threads, Settings, Keyboard shortcuts, Toggle sidebar, Toggle side panel, Add project…, Switch theme. In a thread: Chat ⇄ Terminal, Show changes, Show files, Show canvas, Show subagents, New shell, Rename thread, Fork thread, New side chat. Plus threads, projects, agents ("New thread with …") and files (@) |
| Threads page | Filters: Project, Agent, Status, Sort (Attention, Most recent, Title). Bulk actions: Pin, Move to Draft, Archive, Delete, Clear |
| Side panel `+` menu | Browser, Terminal, Canvas, Diffs, Files, History, Plan, Subagents, Chats, Side chat |
| Pull requests | Repository picker |
| Composer menus | Persona (Default + teammates); Checkout (Local checkout, New worktree, Worktree checkbox, Cloud "Coming soon"); Device; Project (projects, Add folder…, Clone from URL…); Branch; Mode (Ask every time, Auto-edit, Plan, Bypass permissions); Mic (Dictation, Voice settings… / Install local voice…); Attach (Files…, Images…, Folder as context…); Folders (extras, Remove, Add folder…); @ files and / commands |
| Model / agent picker | Search, Models, Starred, agents, effort, Back |
| Bypass confirmation | Enable bypass / Cancel |
| Mid-turn switch sheet | Switch at end of turn / when quiet; Interrupt and switch now; Cancel |
| Prompt-rail hover card | "Prompt N of M" |
| Agent sign-in sheet | Embedded login terminal |
| Inline editors | Rename (F2 or double-click), Automation editor, Team teammate editor |
| Native pickers | Add project, Choose background image, Select clone folder, composer file and folder pickers, canvas export, usage CSV |
| Toasts | "Code copied", background completion headline, automation started or failed, "Could not start the draft" |
| Intro splash | first launch |
| Linux web context menu | WebKit's native menu |

## 4. Empty, loading and error states (~90)

- **Sidebar:** "Add a project to begin", "No threads match", "No threads yet", Failed badge, running spinner.
- **Threads page:** "No threads / Nothing matches these filters."
- **Archived:** "Nothing archived / Archived threads keep their transcripts and worktrees until you delete them."
- **Automations:**
  - "No automations yet / Run a prompt every morning…"; keep-running hint.
  - Inbox: "Inbox is empty / Each automation run lands here…".
  - Editor errors.
- **Usage:** "No usage yet / Token counts and costs…"; "Could not save: …".
- **Pull requests:** "Choose a repository…", "Loading pull requests…", "Couldn't load pull requests" (+ gh hint), "No pull requests / Nothing matches this view yet.", "Checks · loading…".
- **Kanban:** empty text per column: Draft, In progress, Needs you, Review, Done.
- **Team:**
  - "Add a teammate".
  - Editor validation: "Give your teammate a name."; save error: "Could not save: …".
  - Notes: "No standing instructions yet…"; running spinner.
- **Settings:**
  - "No Crew specialists yet"; "measuring…"; "N agent CLI(s) detected"; "Not installed" / "Binary not found on PATH"; "No projects have Bypass enabled yet".
  - "Choose a background image to animate it".
  - Voice: downloading, installed, failed + Try again.
  - Keyboard: "Press keys… (Esc cancels)", "Modified", "Default bindings".
  - Remote: status and account states, last error.
  - Integrations: Composio key hints, toolkit hints, Chrome pairing states, Paper states, Computer use states.
- **Composer:** "Cloning …", "Clone failed: …", "Could not save the pasted image: …", generic error line, "Loading files…", "No matching files", mic busy, "Install local voice in Settings › Voice to dictate."
- **Model picker:** "No agents on PATH", "No models yet", "Nothing matches that search".
- **Home and shell:** error banner, "Add a project first.", "Could not fetch PR #…".
- **Transcript:** blank while loading, "Waiting for the subagent…", "Starting agent…", "No messages yet", "Failed after …" / "Turn failed" + Retry, error rows, "N failed".
- **Side panel:** "No browser tabs.", tab loading spinner, "No subagents yet", subagent failed or running, "No side chats", "No designs yet / Ask for a page…", "No plan yet / When the agent writes a todo list…".
- **Side chat:** intro text.
- **Diffs:** "No thread selected…", "Preparing diff…", "No uncommitted changes" / "Empty commit", error text (**overflows the panel on non-git folders**), "No checks", "No remote: commits stay local", "Committed" / "Nothing to commit" / "… failed: …".
- **History:** "No commits, or this folder is not a git repository." / "No commits match."
- **Files:** "No workspace…", "Empty folder" / "No matches", "No preview", "Could not save: …".
- **Office:** "No slides / This deck is empty.", "(empty)", "This workbook has no worksheets".
- **Browser:**
  - Empty: "Preview your work…" / "Opened in your browser…".
  - Errors: "Couldn't load this page" + Try again, "Could not open this page: …", tool errors, navigation failure messages.
  - Linux: missing WebKitGTK, "The browser process stopped…".
- **Terminal:** "Terminal failed: …", "Shell exited · press Enter to restart", "The agent exited", opening spinner.
- **Palette:** "Nothing matches", "No files (open a thread first)".
- **Remote:** "dove-mesh-host is missing from this build", "couldn't start remote host: …".
- **Other:** voice errors, Mermaid render failure box.

## 5. Routes and panel kinds

- **Routes:** home, threads, kanban, automations, prs (pull-requests), agents, usage, archived, settings, help, team, plus thread and terminal views.
- **Side panel tabs:** Diffs, History, Files, Plan, Browser, Terminal, Side chat, Canvas, Subagents, Chats.
