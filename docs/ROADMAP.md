# Roadmap: making Dove the #1 ADE

Status: **proposal** (2026-10-27). Nothing in the code has changed.
Companion spec: [NOTCH.md](NOTCH.md) (the cross-platform notch).
Related plans: [revamp-v3/PLAN.md](revamp-v3/PLAN.md) (UI and performance), [GROWTH.md](GROWTH.md) (launch), [PRICING.md](PRICING.md).

## 1. Where Dove stands

Dove is already broad. Measured from `main` at 467a1c6: about 86k lines in `apps/dove`, 7 workspace crates, 578 tests, CI and packaging workflows, and 20+ agents supported through Zeron drivers and ACP.

**What's strong (keep and advertise):**

| Area | What exists |
| --- | --- |
| Breadth | Claude Code, Codex, Cursor, Gemini, OpenCode, Devin, Copilot, goose, Qwen, Droid, Amp and more, all mapped onto one `AgentEvent` stream |
| Isolation | A worktree per task, PRs from the thread, commit/push/merge |
| Two ways to drive | Chat ⇄ terminal on one session, and Flock: agent TUIs and shells in split panes and grids |
| Orchestration | Team (teammates with roles), delegation with depth/fan-out limits, permission capping, `task_links` lineage that survives restarts |
| Tools for every agent | One injected `dove` MCP server: browser, generative UI, office docs, computer use, Composio, Paper |
| Ambient control | Voice Mode (local STT, VAD, turn detection), phone remote over a CRDT relay, tray, OS notifications |
| Native performance | Rust + GPUI, about 63 MB idle RSS, idle window renders zero frames, 69 MB binary |

**Gaps I found that block "#1":**

1. **No self-update for Dove itself.** `updates.rs` checks the *agents'* releases; I found nothing that updates Dove. Releases are unsigned (SmartScreen and Gatekeeper warnings). Every other gap matters less than a broken first launch.
2. **The headline wedge isn't built.** GROWTH.md's pitch is "three agents on one task, compare the diffs, merge the best". I found no compare or race view: you can run three threads, but nothing puts their results side by side.
3. **The review loop ends at "look at the diff".** There are no inline review comments that go back to the agent, no per-turn checkpoints to roll back to, and no per-worktree run/test/preview.
4. **No cloud or remote execution.** The composer shows "Cloud · soon". Laptop-lid-closed work is the next big expectation.
5. **Polish debt.** Most of revamp v3 (type scale, motion system, settings rebuild, error states) is still open.
6. **Out of sight, out of mind.** Dove only helps while its window is in front. Phone and voice cover part of this; **the notch is the missing everyday surface** (§3.5, [NOTCH.md](NOTCH.md)).

## 2. What "#1" means

The competition is crowded: agent CLIs with their own apps (the Claude desktop Code tab, the Codex app), editor-first products (Cursor, Zed, VS Code agent sessions, Antigravity), terminal-first (Warp), worktree runners (Conductor and similar), and notch utilities that track agents (Syllo). Dove can't win on any single feature. It wins if one claim is true and easy to prove:

> **Dove is the fastest way to run any agent, many at once, and ship what they produce, without babysitting them.**

Scorecard to track every release (publish it):

| Measure | Target |
| --- | --- |
| Download → first agent reply | < 60 s, no terminal needed |
| Cold start → window | < 400 ms (release) |
| Idle CPU / RSS | < 0.5 % / < 80 MB, zero idle renders |
| Time from "agent finished" to user noticing | < 5 s (notch, OS notification, phone) |
| Agents supported with full permissions/approvals | every major CLI |
| Crash-free sessions | ≥ 99.5 % |

## 3. Pillars and work

Sizes: **S** = days, **M** = 1–2 weeks, **L** = 3+ weeks, for one focused engineer or agent team.

### 3.1 Trust and first run (do first)

| Item | Size | Notes |
| --- | --- | --- |
| Code signing: Authenticode (Windows), Developer ID + notarization (macOS) | M | Blocks the launch in GROWTH.md |
| Dove self-update (signed manifest, delta where possible, "restart to update" pill) | M | Reuse the verified-download code from the voice runtime (pinned SHA-256, `.verified` sidecar) |
| First-run wizard: detect agent CLIs, sign in to one, pick a project, first task | M | Install missing CLIs from inside Dove (the npm/GitHub logic in `updates.rs` already knows the commands) |
| Opt-in crash reporting and an anonymous activation ping | S | GROWTH.md's funnel needs it to be measurable |

### 3.2 Compare and race (the wedge)

| Item | Size | Notes |
| --- | --- | --- |
| "Run on N agents": one prompt fans out to N worktrees (Claude, Codex, Gemini…) | M | `orchestra.rs` already spawns children in worktrees; this is a UI and grouping layer |
| Compare view: diffs side by side, test results, cost and time per agent | L | Builds on `diff_list.rs` and the usage ledger |
| Merge the winner, cherry-pick hunks from the others, archive the rest | M | |
| Optional judge: a teammate reviews all N and recommends one, with reasons | S | It's a Team delegation with a fixed brief |

### 3.3 The review loop

| Item | Size | Notes |
| --- | --- | --- |
| Inline comments on diff lines, sent back to the agent as one batched message | M | |
| Checkpoints per turn (a git ref per turn), and rewind the files plus the conversation | M | Worktrees make this cheap |
| Run, test and preview per worktree: detected scripts, port allocation, a preview in the in-app browser | M | Ports listed in the notch too |
| PR status in the thread: CI checks, review comments pulled back to the agent | M | `pr_view.rs` exists; add the feedback loop |

### 3.4 Remote and cloud workers

| Item | Size | Notes |
| --- | --- | --- |
| SSH hosts: run the harness and worktree on another machine, with UI local | L | The `LiveSession` boundary already isolates the UI from the process |
| Hosted workers (the "Cloud" checkout) | L | Paid tier per PRICING.md; same event stream |

### 3.5 Ambient layer: notch, phone, voice

The notch is a new everyday surface: what every agent is doing, approvals, music, and a drag shelf, at the top of the screen. Full spec: **[NOTCH.md](NOTCH.md)**. It shares one "attention model" with the sidebar, tray, OS notifications, phone and Voice Mode, so all five always agree.

### 3.6 Polish and performance

Finish [revamp v3](revamp-v3/PLAN.md) in its own order: performance first, then tokens, motion and kit, Settings, then the shell and pages. Add a per-PR perf gate (startup, idle renders, RSS) next to the existing binary-size gate.

### 3.7 Context that follows the project

| Item | Size | Notes |
| --- | --- | --- |
| One project rules file, synced to `CLAUDE.md`, `AGENTS.md`, `GEMINI.md` and `.cursor/rules` | S | Switching agents keeps the same instructions |
| Memory (Supermemory) visible and editable per project | M | `supermemory.rs` and `memory_settings.rs` exist |
| Handoff: continue a thread in another agent with a summary and the same worktree | M | |

### 3.8 Ecosystem

Skills and plugins already exist. Add a browsable catalog (skills, MCP servers, teammate templates, automations), one-click install, and "share my setup" export.

## 4. Order

| Phase | Contents | Exit criteria |
| --- | --- | --- |
| **1. Ship-ready** | 3.1, perf part of 3.6, notch N0 spike | Signed builds auto-update; perf scorecard published; notch window works on all three OSes |
| **2. The wedge** | 3.2, notch N1 (agents) and N2 (shelf) | A 30-second "three agents race" video is real; approvals work from the notch |
| **3. Close the loop** | 3.3, 3.7, notch N3–N4 | Review comments and rewind; Now Playing; drag out of the notch |
| **4. Beyond the laptop** | 3.4, 3.8, the rest of revamp v3, notch N5 | SSH and hosted workers; catalog; notch extras |

Phase 1 is the GROWTH.md beta; phase 2 is launch week.

## 5. Decisions needed

1. **Order:** is the notch in phase 1–2 (a launch differentiator), or after the compare view?
2. **Cloud workers:** build hosted, or SSH-only first?
3. **Signing budget:** an EV certificate for Windows (instant SmartScreen reputation) or a standard one?
