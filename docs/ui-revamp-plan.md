# Keplorayx UI/UX revamp — plan v2

Status: proposal, 2026-10-12. Supersedes v1, which is folded in below.
Owner: main UI session (Opus 5.5). Bounded sweeps go to Haiku subagents.
Coordinate with:
- **Model picker** session: `dove-harness/src/models.rs`, the composer model popover, `picker_memory.rs`.
- **Onboarding animation** session: `apps/dove/src/intro/`.

## 0. Context

The user's direction for the revamp:

1. **Extremely fast chat rendering.**
2. **Chat ⇄ Terminal toggle per session.** A session always opens in chat view. One toggle switches to the agent's real TUI for the same session, live, and back again.
3. **Image 9 layout.** A permanent top bar with thread tabs. A collapsible sidebar with New thread, Threads, Kanban, Automations, Pull requests and More.
4. **One composer** combining the four reference input bars.
5. Every v1 improvement, plus more.

## 1. Research findings that shape the design

**The chat ⇄ terminal toggle.** "Superlogical" and "Sentrytron" don't appear to exist. The product that does exactly this is **Agents Room**: a Conversation pane and a Terminal pane over one CLI session. Its chat is a read-only render of the CLI's saved transcript, and it drops to the terminal when the CLI needs its own UI.

Three ways to build it:

| | Technique | Pros | Cons |
|---|---|---|---|
| A | **TUI always**: agent runs in a PTY; chat is built by tailing its transcript file (Agents Room) | Instant switching. One process. | No structured approvals, steering or usage. Claude's JSONL format is undocumented and "changes between versions". Every agent needs its own transcript parser. |
| B | **Headless for chat, `--resume` into the TUI on toggle** | Chat keeps rich control: permissions, steering, usage events. Uses only documented flags. | The toggle restarts the process (about 0.5–1.5 s). It cannot switch mid-turn without interrupting. |
| C | **Hybrid (recommended).** B as the engine, plus a transcript-tail mirror so chat stays live while the TUI runs, plus a pre-warmed PTY | Chat is fully featured. Switching costs about one process start, and the chat view never goes stale. | More code. Agents that can't resume into a TUI fall back to a shell in the worktree. |

Verified facts that C relies on:
- `claude --resume <id>` works on sessions started with `-p`. Their ids are hidden from the picker but resumable by id.
- Running both at once interleaves messages into one transcript, so exactly one process may own a session at a time.
- On resume, MCP config, settings, plugins and `--add-dir` must be passed again.
- Claude Code has native `--bg` / `claude attach`, to be evaluated as an alternative attach path.
- Codex: `thread/resume` over app-server is verified. Whether `codex resume <thread>` works in its TUI must be spiked.

**Fast rendering (from Zeron's Rust code).**
- **Incremental markdown**: re-parse only from the last stable top-level block, with a "mend" pass that closes half-written markers so text never reflows mid-stream.
- **Row preparation off the UI thread** (`TranscriptPreparation`), cached per entry id and invalidated only for changed ids.
- **Analytic heights** for folds and chips. Text rows are measured once, with remeasure hints that keep the scroll anchor.
- **Stick-to-bottom spring** with a growth EMA that parks when idle (no animation loop), and a 70 px release band.
- **Send runway**: space reserved under a just-sent message so it glides to the top.

Keplorayx's biggest current cost: the streaming message re-parses its *entire* markdown on every 8 ms batch (`transcript.rs:317`), which is O(n²) over a long reply.

**Voice.** Zeron dictation is local-only:
- Model: Parakeet TDT 0.6B v3 INT8 ONNX, 670 MB, downloaded from Settings with SHA-256 verification, CC-BY-4.0.
- Runtime: `parakeet-rs` + `ort` (CPU). Capture: `cpal`, resampled with `rubato`.
- Interaction: hold to talk or tap to toggle, final-only transcripts, never auto-sends.

**Meters.**
- Context ring: from harness events (Claude: latest message input + cache tokens; Codex: `tokenUsage.last`; ACP: `usage_update`). Colors: muted < 75%, warning ≥ 75%, danger ≥ 90%.
- Plan-usage ring: from rate-limit windows. Claude has `rate_limit_event` and Codex has `account/rateLimits/updated`. Keplorayx currently ignores both.

**Kanban (Synara).** Cards are chats. Columns: Draft → In progress → Awaiting you → Done, with a Review state when a live PR exists. Cards support "Send as goal".

**Automations (Codex, Capy).**
- Triggers: schedules (RRULE or cron) and event triggers.
- Each run starts a new session, either in the project checkout or in a dedicated worktree.
- An inbox shows runs with findings.
- Desktop automations need the app running.

## 2. Principles (unchanged)

Attention first · keyboard-complete · calm by default · honest (no fake buttons; disabled items say why) · **zero idle cost**. Budgets: idle CPU ≈ 0 renders/s, RAM < 60 MB idle, startup < 300 ms, 120 fps while streaming three sessions.

## 3. Layout (image 9)

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ ◈ ▣ ‹ › │ [● Fix login ×] [◌ Add tests ×] [+]               ⚙ ─ □ ×          │ ← top bar, always
├──────────────┬───────────────────────────────────────────────────────────────┤
│ ✎ New thread │                                                               │
│ ☰ Threads    │   (route content: chat / terminal / board / automations /     │
│ ▦ Kanban     │    pull requests / settings)                                  │
│ ⟳ Automations│                                                               │
│ ⇅ Pull reqs  │                                                               │
│ ⋯ More       │                                                               │
│──────────────│                                                               │
│ Needs you  2 │                                                               │
│ Threads 🔍 ⋯ │                                                               │
│  ● Fix login │                                                               │
│  ◌ Add tests │                                                               │
│──────────────│                                                               │
│ ◉ Local · RAV│                                                               │
└──────────────┴───────────────────────────────────────────────────────────────┘
```

**Top bar** (always visible, including when the sidebar is collapsed):
- Keplorayx mark, sidebar toggle (Ctrl+B), back/forward.
- **Thread tabs**: open threads with a status glyph and title. Middle-click closes; closing doesn't archive; drag to reorder. A **+** opens a new thread.
- Right side:
  - **per-thread view switch `Chat | Terminal`** (Ctrl+`) when a thread is active;
  - panel toggles (Changes / Files);
  - session-info popover;
  - window controls.

**Sidebar** (collapsible to 0, resizable 208–400 px, state persisted):
- **Nav**:
  - New thread (Ctrl+N).
  - Threads: a full list page with filters by project, agent and status, plus bulk archive.
  - Kanban.
  - Automations, with an unread dot when runs have findings.
  - Pull requests.
  - More: Agents, Usage, Archived, Settings, Help and shortcuts.
- **Needs you**: pinned, with a count. Rows show the pending approval or question with inline Allow/Deny.
- **Threads**:
  - Search and an options menu (group by project, density, show archived).
  - Attention-sorted: Needs you → Working → Unread done → Failed → recent. Pinned first.
  - Rows: status glyph (animated spinner, check, amber, red) · agent mark · title (bold if unread) · time, or archive on hover.
  - Rows glide 260 ms when they resort.
- **Footer**: the device ("Local · RAVTHEMAN"), Usage today, and Settings.

**Command palette (Ctrl+K)**: threads, projects, actions, agents, files (`@`), settings. Every action in this plan is reachable from it.

## 4. The composer (images 10–12 + Zeron)

```
 ┌ Local ▾ ┐ ┌ ▣ dove ▾ ┐ ┌ ⎇ main ▾ │ ☑ worktree ┐                    ◔ 38%  ◔ 6%
╭──────────────────────────────────────────────────────────────────────────────╮
│ Ask anything, / for commands, @ for context                                  │
│ [📎 screenshot.png ×] [@src/main.rs ×]                                      │
│                                                                              │
│ ＋  🎙▾   ⛨ Ask every time ▾              ✳ Opus 5.5 · Medium ▾          (↑) │
╰──────────────────────────────────────────────────────────────────────────────╯
```

- **Context row** (above the pill):
  - **Local/Cloud** device chip. Cloud shows "coming soon" until the cloud phase.
  - **Folder/project** picker: search, Add folder…, Clone URL….
  - **Branch** picker with a **worktree** checkbox (on by default for new threads).
  - **Add folder** (extra directory, `--add-dir`).
  - In an existing thread, the row collapses to read-only chips: branch · worktree.
- **Right of the context row: two rings.**
  - **Context window %**: hover shows tokens used/limit and model.
  - **Plan usage %**: hover shows the 5-hour and weekly windows with reset times. Click opens Usage.
- **Input**:
  - Placeholder "Ask anything, / for commands, @ for context".
  - Auto-grows from a 49 px compact pill (Zeron hysteresis) to a 300 px cap.
  - `@` mentions files from the worktree; `/` offers agent slash commands plus Keplorayx templates.
  - Paste or drop images and files; attachments show as chips with previews.
- **Bottom row**:
  - **＋** attach (files, images, folder context).
  - **🎙** dictation: hold to talk or tap to toggle. The ▾ menu chooses the model or "Install local voice…".
  - **Permission level**: Ask every time / Auto-edit / Plan / Bypass permissions. Bypass shows a red tint and a confirmation the first time per project.
  - **Agent · Model · Effort**: the Model picker session's popover, slotted in.
  - **Send / Queue / Steer / Stop**: one button whose shape changes with state.
- **While a run is live**:
  - Typed text **Steers** (Claude, Codex) or **Queues** (others). Queued messages show as reorderable chips above the input.
  - Esc with an empty input interrupts.
- **Question panel** replaces the composer when the agent asks (AskUserQuestion / requestInput). Paged "1/3"; number keys select; free-text override.
- Drafts and attachments persist per thread across navigation.
- Dictation in the terminal view types into the PTY.

## 5. Chat ⇄ Terminal (technique C in detail)

**Ownership model.** Each live session has exactly one **driver**: `Headless` (structured adapter) or `Tui` (PTY). The engine enforces it.

**Toggle to Terminal (Ctrl+`):**
1. If the session is **idle**, stop the headless process (it is between turns, so nothing is lost). Spawn the agent's TUI in a PTY with resume args, in the thread's worktree, passing the same model, permission mode, MCP, settings and add-dirs. Show the terminal immediately with a 300 ms skeleton, swapping in the first frame.
2. If the session is **mid-turn**, show a sheet:
   - **Switch at end of turn** (default): the toggle arms and fires on `TurnEnded`.
   - **Interrupt and switch now**.
3. **Pre-warm**: when the user hovers the toggle or focuses a thread with "terminal" as its last view, start resolving the binary and preparing the PTY (no process start).

**While in the TUI:**
- A **transcript mirror** tails the agent's session file:
  - Claude: `~/.claude/projects/<slug>/<id>.jsonl`.
  - Codex: its rollout file.
- The mirror feeds the same `AgentEvent` stream through a tolerant, versioned parser. Lines it can't parse are skipped and logged.
- Chat, the sidebar status, the diff and the context ring all stay live.
- The chat view shows a banner: "Running in terminal · read-only here · Switch back".

**Toggle back to Chat:**
- If the TUI is idle (no output for 1 s and a prompt visible), send `/exit` (Claude) or Ctrl+D, wait up to 2 s, then kill the PTY.
- Resume headless with `--resume <id>`; the chat continues seamlessly.
- If the TUI is busy, the same end-of-turn or interrupt choice applies.

**Per-agent support:**

| Agent | Terminal mode |
|---|---|
| Claude Code | Full: `claude --resume <id>`. Evaluate native `claude attach` / `--bg` as the attach path. |
| Codex | `codex resume <threadId>` (spike in P0). Fallback: a shell in the worktree. |
| OpenCode, Cursor, Pi, ACP agents | Their own TUI with resume where supported (`opencode --session`, `cursor-agent --resume`, `pi --session-id`); otherwise a shell in the worktree with the resume command typed in, plus a note. |
| Shell harness | Terminal-only by nature. |

**Engineering:**
- `dove-term::Pty::spawn_command(cwd, CommandBuilder, cols, rows)` (today it only spawns a shell).
- `TerminalView::for_command(...)`.
- Each thread keeps its terminal entity alive across toggles, so scrollback survives.
- Plain shell terminals stay as separate tabs in the Terminal panel.

## 6. Fast chat rendering

1. **Incremental markdown** (port of Zeron `IncrementalParser::set_text` and `mend`): re-parse from the last stable block, append new blocks, splice only the tail rows. One assistant message becomes **one row per top-level block**, so a 2,000-line answer is about 200 cheap rows, not one giant row.
2. **Background preparation**: events → row diffs computed on the background executor (parse, highlight, layout hints). The UI thread only splices `ListState` ranges. One notify per frame (already batched at 8 ms).
3. **Syntax highlighting**: tree-sitter grammars loaded lazily per language, highlighted off-thread, cached by content hash. Shared by code blocks, diffs and file preview.
4. **Stick-to-bottom spring** with an EMA and an idle park; a 70 px release band; a "Jump to latest" pill.
5. **Send runway**: the user's message glides to the top as the answer streams in.
6. **Shaped-line cache** keyed by (style, text) for repeated chips and labels. Overdraw 320 px.
7. **Cold open**: render the last screenful from a cached tail of about 64 entries first, then hydrate the rest in the background. The target is under 16 ms to the first frame of a 10k-row thread.
8. **Measurement**: `DOVE_FRAME_STATS`, plus a `dove-bench` replay of recorded streams at 10× speed that asserts frame time under 8 ms.

## 7. Conversation polish

- **Activity fold**: "Thought for 12 s · Ran 3 commands · Edited 2 files". Open while streaming, folded when done, click to pin.
  - **Run**: command, exit-code chip, last 3 output lines, expandable.
  - **Edit**: path plus a mini diff, with "View in Changes".
  - **Read / Search**: path, opens a preview.
  - **Subagent**: nested activity.
  - A guide rail connects the steps; failures get an error tint.
- **Answers**:
  - Monochrome markdown (neutral inline-code wash, not accent).
  - Clickable `path:line` opens the file preview.
  - Per-block copy.
  - Hover actions: Copy · Retry · Fork from here.
- **User bubble**: hover shows the time, Copy, and Edit & resend (fork).
- **Message rail** (minimap) on the left, with a preview on hover.
- **Working indicator** in a reserved status strip: animated glyph · live step ("Editing src/main.rs") · elapsed.
- **Permission card**:
  - Command or diff preview with risk hints (outside the worktree, network, delete).
  - **A** Allow · **S** Allow for session · **D** Deny hotkeys.
  - Mirrored into Needs you.
- **Auto titles**: 3–6 words after the first answer. Rename inline with F2.
- Per-turn token lines are removed. Usage moves to the rings and hover.

## 8. Pages

- **Threads**: a table or list of all threads with filters, multi-select (archive, delete, move to Kanban column), sort, and search.
- **Kanban**:
  - Columns: **Draft · In progress · Needs you · Review · Done** (Review = an open PR exists for the thread's branch).
  - Cards: title, agent mark, project, branch, ±diffstat, PR or CI chip, elapsed.
  - Drag Draft → In progress starts the thread with its draft prompt. Drag to Done archives after confirmation.
  - "+ Draft" in any column.
  - Agents can't move cards in v1.
- **Automations**:
  - List of automations, each with name, prompt, project, agent/model, trigger, worktree mode and enabled state, plus a "Run now" button.
  - **Schedule**: presets or custom cron/RRULE, in the local timezone.
  - **Event triggers**: "on new commit to branch" and "on PR opened/review requested" (via `gh` polling).
  - Each run creates a normal thread tagged with the automation.
  - The **Inbox** tab lists runs with unread markers.
  - The scheduler lives in `dove-core` and runs while Keplorayx runs. An optional tray icon keeps it alive with the window closed.
- **Pull requests**: the existing list plus a detail page (description, checks, files using the diff view) and **Check out in new thread**.
- **More**:
  - **Agents**: per agent, path (override), version, sign-in status with "Sign in", default model, and enabled.
  - **Usage**: by day, agent and project, with export to CSV.
  - **Archived**.
  - **Settings**, rebuilt with left navigation: General · Appearance · Composer · Voice (install or remove the local model, input device, shortcut) · Keyboard (recordable) · Notifications · Data · About.

## 9. Side panel

Tabs: **Changes · Files · Terminal (shells) · Preview · Plan**. Resizable 360–760 px, remembered per thread.

- **Changes**:
  - Stage, discard, unified or split view, intra-line highlights.
  - Line comments sent to the agent as a batched review.
  - Footer: **Commit → Push → Open PR**, with message, title and body drafted from the thread, then CI status and Merge.
- **Files**: tree with search and an inline syntax-highlighted preview; "Open in editor".
- **Terminal**: multiple shell tabs; localhost URLs become links to Preview.
- **Preview**: system browser in v1; a lazily created webview window later.
- **Plan**: the agent's todos, with a progress chip mirrored in the status strip.

## 10. Notifications and attention

- OS notification when a thread needs you, finishes, or fails while Keplorayx is unfocused; clicking opens the thread.
- Taskbar badge with the Needs-you count. Optional sound.
- In-app toasts for background completions.

## 11. Foundation (built first)

- **Kit** (`apps/dove/src/kit/`): Button, IconButton (tooltip + shortcut), SegmentedControl, Tabs (draggable), Menu and ContextMenu (keyboard, submenus), Popover, Dialog/Sheet, Toast, Tooltip, Kbd, TextField, Switch, Checkbox, Badge, StatusGlyph, Ring, Skeleton, EmptyState, ResizeHandle.
  - Behavior comes from `gpui-base` where possible.
- **Tokens**: spacing, radii, type scale, elevation, plus `state.*` colors.
- **Motion tokens** (Zeron catalog), gated by reduced motion and frame-requesting only while animating.
- **AppState entity**: projects, threads with derived attention, unread, open tabs, layout, drafts, view mode per thread. Child views (TopBar, Sidebar, Main, SidePanel, StatusStrip) are cached so a stream in one doesn't rebuild the others.
- **Engine summaries**: last activity, current step, diffstat, unread, context %, and rate-limit windows. Parse Claude `rate_limit_event` and Codex `account/rateLimits/updated`. Add a `ContextUsage` event.
- **Bug fixes**:
  - B1: launches minimized.
  - B2: the composer shows the default model, not the session's.
  - B3: idle CPU.
  - B4: startup time.
  - B5: missing a11y roles.
  - Terminals leak after a thread is deleted.
  - `fork_task` drops the mode and effort.

## 12. Phases

| Phase | Scope | Exit criteria |
|---|---|---|
| **P0 Foundation** | Kit, tokens, motion, AppState, cached views, engine summaries, bugs B1–B5. Spikes: Claude `--resume` TUI handoff, `claude attach`, `codex resume`, transcript tail. | 0 idle renders/s, startup < 400 ms, handoff spike works for Claude |
| **P1 Fast chat** | Incremental markdown, block rows, background preparation, highlighting, spring and runway, cold-open tail, bench | Long Claude reply streams at < 8 ms/frame; 10k rows open in < 16 ms |
| **P2 Shell and navigation** | Top bar with thread tabs, new sidebar (nav, Needs you, attention sort, search, resize), command palette, notifications | Triage five sessions keyboard-only |
| **P3 Composer** | Context row, rings, attachments, @ and /, permission level, steer/queue, question panel, drafts | Every control in image 12 works |
| **P4 Chat ⇄ Terminal** | Driver ownership, PTY command spawn, handoff both ways, transcript mirror, per-agent table | Toggle on an idle Claude thread < 1.5 s; chat stays live during the TUI |
| **P5 Conversation polish** | Activity fold with mini diffs, actions, rail, working indicator, permission hotkeys, auto titles | A long run reads cleanly; approvals take one key |
| **P6 Pages** | Threads, Kanban, Automations (scheduler + inbox), PR detail, More pages | An automation fires on schedule into a thread |
| **P7 Side panel and delivery** | Changes with stage, comments and the commit/push/PR footer, file preview, plan | Prompt → review → PR inside Keplorayx |
| **P8 Voice** | Parakeet local install from Settings › Voice, mic in the composer and terminal | Dictate offline; installer verifies hashes |
| **P9 Polish** | Light-theme parity, empty states, skeletons, motion review, a11y sweep | Visual QA checklist clean |

**Voice and the size budget.** The 670 MB model and the ONNX runtime are downloaded on demand from Settings, never bundled, so the installer stays under 30 MB.

## 13. Work split

- **Opus**: the kit, shell, composer, fast transcript, the terminal handoff engine, pages and all visual design.
- **Haiku subagents**:
  - transcript-format fixtures and mirror parser tests per agent;
  - icon SVGs in the Solar linear style;
  - a11y role/label sweep;
  - settings copy and the shortcut table;
  - screenshot capture runs;
  - cron/RRULE test cases.
- **Model picker session**: the agent/model/effort popover, consumed by P3.
- **Onboarding session**: the intro, coordinated with P0 startup work.

## 14. Verification

- **Unit tests**: incremental parser vs full parse (equivalence on random prefixes), attention sort, cron next-fire, transcript-mirror parsers against recorded fixtures, driver state machine (idle and mid-turn handoffs), keystroke and permission hotkeys, ring thresholds, contrast checks.
- **GPUI view tests**: sidebar order, tab strip, question paging, queue chips.
- **Live**: Claude chat → terminal → chat round trip on a real repo with history intact; Codex the same, or the documented fallback; an automation firing; a PR from the Changes footer.
- **Perf**:
  - `perf.ps1` for startup, RAM and idle CPU.
  - `DOVE_FRAME_STATS` for renders/s while idle and while streaming three threads.
  - `dove-bench` for frame time.
- **Visual**: per-phase screenshot sets in dark and light, compared against reference images 2–12.

## 15. Decisions (confirmed by the user, 2026-10-12)

1. **Terminal technique**: hybrid C (headless chat, `--resume` handoff to the TUI, transcript mirror while in the TUI).
2. **Mid-turn toggle default**: switch at end of turn; "interrupt now" stays available.
3. **Automations while closed**: opt-in tray icon keeps the scheduler running.
4. **Cloud chip**: shown, disabled as "coming soon".

## 16. Open risk found during research

`crates/dove-harness/Cargo.toml` now depends on `zeron-harness` and `zeron-proto` by relative path (`../../../zeron/...`), added by the model-picker session. Keplorayx won't build without a sibling Zeron checkout, which breaks CI and any fresh clone. Before P0 lands, either vendor the needed Zeron crates under `vendor/` (MIT, with notices) or pin them as a git dependency at a fixed revision.
