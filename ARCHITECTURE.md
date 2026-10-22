# Dove architecture

Dove is a native desktop Agentic Development Environment written in Rust. It does not ship a model or an agent of its own. It launches the coding-agent CLIs a developer already has (Claude Code, Codex, Cursor, Gemini CLI, OpenCode, Devin and others), turns their output into one event stream, and wraps each task with a git worktree, a transcript, a diff, a browser, files and a terminal.

This document describes how the pieces fit. Paths are relative to the repository root.

## Workspace layout

```text
apps/dove              GPUI desktop app (the `dove` binary)
apps/landing           Static marketing site (no build step)
crates/dove-core       Engine: runtime, live sessions, task lifecycle, automations, TUI mirror
crates/dove-harness    Agent adapters: Claude/Codex/… via Zeron's drivers, everything else via ACP
crates/dove-proto      Shared types: AgentEvent, HarnessKind, permissions, usage, models
crates/dove-store      SQLite + JSONL: projects, tasks, transcripts, usage, settings, automations
crates/dove-git        Worktrees, status, diffs, history, GitHub PRs through `gh`
crates/dove-term       Terminal emulator and PTY sessions
vendor/                gpui-base, zeron-proto, zeron-harness, zeron-syntax, zeron-voice(-media)
```

The UI toolkit is GPUI from [zeronsh/zui](https://github.com/zeronsh/zui) (Zed's GPUI with backdrop blur, edge fades and rounded clipping), paired with `gpui-base` from zeronsh/gpui-component. Both are pinned to one revision in the workspace `Cargo.toml`. `vendor/README.md` lists every change made to vendored code.

## The app (`apps/dove`)

| Module | Role |
| --- | --- |
| `main.rs` | Process entry. `dove --mcp-browser` runs the stdio MCP bridge instead of the window. |
| `shell.rs` | Root window entity: routes (chat, terminal, pages), top bar, panel layout, wiring to the engine. |
| `sidebar.rs` | Projects and threads, attention-sorted (needs you, working, unread, failed, recent). |
| `composer/` | Input pill, model/agent/effort picker, permission mode, attachments, queue/steer. |
| `transcript.rs`, `markdown*.rs`, `math.rs` | Incremental transcript rendering: markdown, tables, Mermaid, math, images, folded tool calls. |
| `widget.rs` | Generative UI cards (` ```ui ` fences and the `show_ui` tool). |
| `side_panel.rs`, `changes.rs`, `history.rs`, `files_panel.rs` | Per-thread side panel: diff review, git history, file viewer/editor. |
| `browser/` | Native web view per thread (WKWebView, WebView2, WebKitGTK) with GPUI chrome and the agent driver. |
| `terminal/` | Terminal view over `dove-term`, also the host for an agent's own TUI. |
| `side_chat.rs` | A second agent in the same worktree as the parent thread. |
| `pages/` | Threads, kanban, automations, usage, agents, archived, help. |
| `settings.rs`, `theme.rs`, `kit.rs`, `ui.rs` | Settings, light/dark theme tokens, shared glass/frost primitives. |
| `mcp_browser.rs` | The `dove` MCP server injected into every harness. |
| `home.rs`, `wallpaper.rs` | New-thread page: the user's wallpaper through a retro effect (Original, Dither, Halftone, Scanlines), optionally animated. |

### Backdrop effects and animation (`wallpaper.rs`)

The wallpaper is decoded once, cover-cropped to the hero's device-pixel size and rasterized on the background executor, so a dither or halftone cell stays the same few screen pixels on every display. The dither is ordered 4×4 Bayer dithering on 2×2 device-pixel cells: brightness is the brightest channel, the threshold is `(m + 0.5) / 16`, on cells are normalized so their brightest channel reaches 255, and off cells keep 8% of their color, so hue is preserved.

**Animate backdrop** (setting `wallpaper_animate`) gives each effect its own motion, all derived from the image itself so it works on any wallpaper. A one-time analysis finds a focal point (the brightest blurred region, nudged toward the center) and two dominant hues (falling back to blue and orange for grey art).

| Effect | Motion | Where it runs |
| --- | --- | --- |
| Original | Slow Ken Burns push toward the focal point, 20 s loop | GPU only: a paint transform (`Motion`), no re-raster |
| Dither | An aircraft crosses toward the focal point with twin contrails in the image's hues; slow brightness waves move through the signal while the Bayer grid stays fixed; seamless 24 s loop | CPU, 24 fps |
| Halftone | Dot radius ripples outward from the focal point | CPU, 15 fps |
| Scanlines | The line pattern rolls, a soft band sweeps down, faint flicker (flicker is paint opacity) | CPU, 24 fps |

Frames render on the background executor, one at a time; each finished frame refreshes the window, and the next is requested only from the home page's render, so animation stops whenever the home page is not drawn. Replaced frames are freed from GPUI's sprite atlas with `Window::drop_image`. The Dither frame precomputes each cell's on-color once per image (a uniform gain cancels out of it), so a frame is mostly a multiply and a compare per cell. GPUI exposes no custom-shader hook, so a full-resolution GPU path would need a new zui scene primitive.

## Data flow: engine → `LiveSession` → UI

```text
 composer ──Command──▶ LiveSession.commands ──▶ dove-harness adapter ──stdin/JSON-RPC──▶ agent CLI
                                                                                      │
 transcript ◀──AgentEvent── LiveSession.events ◀── engine recorder ◀──AgentEvent───────┘
                                                     │
                                                     ├─▶ sessions/<task>.jsonl (append)
                                                     └─▶ SQLite: task status, usage rows
```

1. `Engine::start_task` (in `crates/dove-core/src/lib.rs`) resolves the harness binary, optionally creates a worktree with `dove_git::create_worktree_from`, inserts the task row and calls `dove_harness::spawn` with a `SessionSpec` (program, cwd, model, reasoning, permission mode, resume id, first prompt).
2. The adapter runs on the engine's tokio runtime and emits `AgentEvent`s on a `flume` channel. Flume is executor-agnostic, so GPUI's foreground executor drains it without touching tokio.
3. The engine sits between the adapter and the UI. For every event it appends to the task's JSONL transcript, updates status (`next_status`) and usage in SQLite, then forwards the event to `LiveSession::events`.
4. The UI drains ready events once per frame and only sends `Command`s back (`Send`, `Interrupt`, `Permission`, `Answer`, `Kill`). It never writes session activity to the store itself, so a crash cannot leave the transcript and the UI disagreeing.
5. On restart, `Store::read_transcript` replays the JSONL file; a torn final line is skipped rather than fatal.

### Chat and terminal are one session

A session has exactly one driver at a time (`Driver::Chat` or `Driver::Terminal`), because two processes writing one agent session interleave its transcript. Switching to the terminal calls `Engine::claim_terminal`, builds the agent's own TUI command with `--resume <native id>` (`dove_harness::tui`), passes the MCP config again, and starts a `mirror::MirrorSession` that tails the agent's own transcript file (Claude's `~/.claude/projects/…/<id>.jsonl`, Codex's rollout file). Mirrored lines are recorded like live events, so the chat view never goes stale. Switching back releases the terminal and resumes the structured driver.

## Harness boundary (`crates/dove-harness`)

Everything an agent does crosses one boundary: `spawn(rt, SessionSpec) -> SessionHandle`. Each adapter maps its CLI's wire format onto the same `AgentEvent`s (text deltas, reasoning, tool calls and results, plan/todos, permission requests, questions, usage, rate-limit windows, exit).

| Path | Agents | Wire |
| --- | --- | --- |
| `external.rs` → `vendor/zeron-harness` | Claude Code, Codex, Cursor, OpenCode, Pi, Devin, Grok, Hermes, Antigravity (Kilo rides the OpenCode driver) | Claude stream-json, Codex app-server JSON-RPC, Zeron's per-agent drivers |
| `acp.rs` | Gemini CLI, Copilot, goose, Qwen Code, Factory Droid, Cline, Kimi, Amp | Agent Client Protocol v1 over newline-delimited JSON-RPC |

`detect.rs` finds installed CLIs on `PATH` (plus well-known install locations), `catalog.rs` refreshes each agent's live model list, and `models.rs` maps Dove's model/effort choices onto each CLI's flags. Dropping a `SessionHandle` kills its child.

## MCP injection

Dove gives every agent the same extra tools by registering one MCP server, named `dove`, with every session it starts:

- `main.rs` calls `dove_harness::set_tool_server` with `{command: <dove exe>, args: ["--mcp-browser"], env: [port, token]}`.
- Claude Code receives it through a generated `--mcp-config` file and `--allowedTools mcp__dove` when it runs in its TUI (`tui.rs`); the Zeron drivers (Claude, Codex and the rest) through `zeron_proto::McpServer` (`external.rs`); ACP agents through `mcpServers` on `session/new` and `session/load` (`acp_mcp_servers`).
- `is_dove_tool` recognises the tool names however a harness spells them (`mcp__dove__browser_click`, `dove/browser_click`, …) so the transcript can render Dove's own tool calls natively.

Tools exposed today: the browser set (`browser_navigate`, `browser_snapshot`, `browser_click`, `browser_type`, `browser_hover`, `browser_press`, `browser_scroll`, `browser_screenshot`, …) and `show_ui` for generative cards. New integrations (Composio, Paper, office documents, computer use) register through this same server so they work in every harness without per-agent code.

### Pre-warmed sessions

`Engine::prewarm` starts the Claude Code harness with no prompt while the user types in the composer (and `prewarm_resume` for a reopened thread). The Claude driver spawns the CLI immediately but holds its first stdin line on a one-shot slot keyed by `dove.deferPrompt`, so process start, settings and MCP servers overlap with typing. `start_task` / `resume_task` adopt the warm session when harness, folder, model, effort, options, mode and resume id match; otherwise it expires after four minutes.

## Browser control path

```text
agent ─tools/call─▶ dove --mcp-browser (stdio) ─JSON line over 127.0.0.1─▶ browser::control
      ◀─result + PNG─                           ◀─────────────────────────  (token-checked)
                                                                               │
                                                     thread whose cwd matches ─┘
                                                     browser::agent → page_api.js / CDP
```

1. The bridge (`mcp_browser.rs`) is a blocking stdio MCP server; it forwards each call as one JSON line `{token, cwd, tool, args}` to a loopback port opened by the running app.
2. `browser/control.rs` checks the random per-launch token (which only reaches agents through the MCP server's environment) and routes the request to the browser of the thread whose working directory matches the agent's `cwd`.
3. `browser/agent.rs` snapshots the page into an outline with stable refs (`[ref=e12]`), moves a visible cursor and performs the action. On Windows input goes through the DevTools protocol so pages receive trusted events; elsewhere it falls back to `page_api.js`. Screenshots return as base64 PNG the model can see.

The Chrome-extension path reuses this protocol: `chrome_*` calls go from `browser/control.rs` over a second loopback listener (a minimal RFC 6455 WebSocket) to the Dove for Chrome extension (`apps/extension`), which executes them in the user's own browser tab instead of the in-app web view.

```text
dove --mcp-browser ─chrome_*─▶ browser::control ─ws://127.0.0.1/extension─▶ background.js ─▶ page.js (isolated world)
                                 │ token + pinned extension origin             │ validates every message
                                 │ upload paths must be inside the thread cwd  │ reviews in review.html (extension origin)
```

- **Who may connect.** Only `chrome-extension://` origins that present the pairing token (64 hex characters from the OS CSPRNG via `uuid` v4). After the first pairing the extension's origin is pinned in `~/.dove/extension.json`; Settings › Plugins › Dove for Chrome › New pairing code rotates the token and forgets it. Frames are capped at 16 MB.
- **What the extension accepts.** `lib/policy.js` checks every message from Dove (type, tool, the tool's own arguments, 1 MB cap) and every message from inside the browser (only the extension's own pages may pair, resume or decide; injected page scripts may only Stop).
- **Consent.** Host access is optional and granted per site from a click in the extension's review window. Consequential clicks (submit, buy, send, delete, …), Enter in a form and every upload wait in `lib/review.js` for **Allow once**. Password, payment, identity and one-time-code fields are refused. Page text is returned framed as untrusted data.
- **Stop.** Bumps an epoch that every in-flight call checks while it waits; pending reviews are denied. Measured: an in-flight call fails 35 ms after Stop.

Which browser agents use is a setting (Settings › Plugins › ego lite › Agents browse with): ego lite (default when installed), Dove's browser, or both. ego lite is a CLI agents run themselves through the `dove-browser` skill; Dove for Chrome is for work that needs the user's own signed-in browser. The extension never touches ego lite's Spaces.

## Store (`crates/dove-store`)

Everything lives under `~/.dove` (override with `DOVE_HOME`):

```text
~/.dove/
  dove.db                SQLite: projects, tasks, usage, settings, automations, automation_runs
  sessions/<task>.jsonl  append-only AgentEvent transcript per task
```

- `tasks` holds title, harness, model, status, worktree, branch and the agent's native session id (used for `--resume` and forks).
- `usage` keeps one row per turn with input/output/cache tokens and cost (exact when the agent reports it, `estimated` otherwise), which powers the cross-agent usage ledger.
- `rusqlite` is built `bundled` and pinned to Zeron's version so both dependency graphs link one SQLite.

## Remote control: desktop ⇄ Cloudflare edge ⇄ iOS / Android

Status: the desktop host, the `dove-mesh-host` sidecar (`mesh/crates/host`), the Rust mobile core (`mesh/`), the iOS app (`apps/ios`) and the Android app (`apps/android`) are in the tree. The edge is the Cosmos edge, which Dove reuses as-is; set `DOVE_EDGE_URL` to point at your own deployment.

On the desktop, `apps/dove/src/remote.rs` spawns `dove-mesh-host` and talks to it in JSON lines over stdin/stdout. The sidecar owns the Woozlit session and the edge connections. When it is switched off or signed out, nothing leaves the machine. When it is on, it publishes every project and session to the account's registry room and each session's transcript to its own room. It forwards the commands a phone queues back to `remote.rs`, which runs them on the same session the desktop shows.

```text
 Dove desktop                    Cloudflare (Workers + Durable Objects)              iOS / Android
 ────────────                    ──────────────────────────────────────              ─────────────
 engine events ─▶ SessionDoc ─ws─▶ SessionRoom DO (one per session)  ◀─ws─ SessionDoc ─▶ transcript UI
 (Loro CRDT)                       RegistryRoom DO (per account: rows) ◀─ws─ thread list
 presence      ─────────────ws─▶  DeviceRoom DO (per device: liveness, nudges)
 commands      ◀────────────────── command frames (send/steer/approve) ◀── composer, approvals
                                   R2: attachments / blobs (optional)
```

- **Identity.** Desktop and phone sign in with the same Woozlit account. On the desktop, "Sign in with Woozlit" opens the browser and gets the token back through a one-time `127.0.0.1` callback that is checked with a `state` value. No client secret ships in any binary. The edge verifies the access token on every WebSocket upgrade and scopes all rooms to that account.
- **Session documents.** Each live session is mirrored into a Loro CRDT document (transcript entries, status, pending permission requests, usage). The desktop is the writer for agent output; phones write only command intents. CRDT merge means a phone that was offline converges without a custom sync protocol.
- **Registry rows.** A per-account `RegistryRoom` holds one row per thread (title, agent, status, updated_at, device), so a phone can list threads without loading every document.
- **Commands.** The phone sends `send`, `steer`, `interrupt`, `permission` and `answer` frames. The desktop applies them through the same `LiveSession::send` path the composer uses; the agent always runs on the desktop.
- **What leaves the machine.** Session events and command frames only. Repositories, worktrees and files stay local unless an agent puts their content into its own output.
- **Cost.** The edge runs on Cloudflare's free tier for individual use. Enterprises can deploy the same Worker into their own Cloudflare account.

## Voice: dictation and Voice Mode

**Dictation** (`apps/dove/src/voice.rs`) is speech to text into the composer: NVIDIA Parakeet TDT 0.6B v3 through the vendored `zeron-voice` crate, downloaded on demand from Settings › Voice into `~/.dove/voice` and verified file by file (SHA-256). Hold the mic to talk, or click to toggle; text is inserted, never sent.

**Voice Mode** is a hands-free call with every thread (`crates/dove-voice`, wired in by `apps/dove/src/voice_call.rs`). It sits above the harness boundary: it reads Dove's own state and acts through the same calls a click makes, so it works with every agent without per-harness code.

```text
 mic ─cpal─▶ voice-call thread ───────────────────────────────────────────────▶ Event::Turn(text)
            resample 16 kHz → NLMS echo cancel (speaker reference) → Silero VAD           │
            → Listener: start / barge-in / pause → Smart Turn v3 (is the turn over?)       ▼
            → speculate at 128 ms of pause ─▶ voice-stt thread (Moonshine | Parakeet)   Conductor
                                                                                       │ fastpath grammar
 speaker ◀─cpal─ voice-speaker thread ◀─ voice-tts thread (system voice) ◀─ clauses ◀──┤ rules router
                 (play / stop in one buffer; feeds the echo canceller)                 │ intent schema + validate
                                                                                       │ safety (destructive?)
                                                                                       ▼
 Shell (voice_call.rs): TaskView::submit · interrupt · decide_from_outside · open_task · Engine::start_task
```

- **Engines are traits** (`engines.rs`): `SpeechToText`, `TextToSpeech`, `TurnDetector`, `EchoCanceller`, `Vad`. Local defaults: Moonshine base (MIT, 67 MB) for recognition, with Parakeet as the accurate option; Silero VAD v5 (MIT, 2 MB) behind an energy gate; Smart Turn v3.2 (BSD-2-Clause, 9 MB) with a pause fallback; the operating system's voice for speech; an NLMS echo canceller. Optional OpenAI-compatible speech-to-text and text-to-speech are off by default, and Settings says what each would upload. Every model is pinned to a Hugging Face revision and SHA-256 (`models.rs`) and re-hashed before ONNX Runtime loads it. Nothing downloads without a click.
- **Threads.** Audio never touches the UI thread: `voice-call` (devices, VAD, turn logic, controls), `voice-stt`, `voice-tts`, `voice-speaker`. The app polls the call's event channel every 30 ms while a call runs.
- **Latency tricks.** The pause is transcribed speculatively after 128 ms, so the text is usually ready when the turn detector decides; a speculation is discarded if speech resumes. Replies are spoken clause by clause (`speakable.rs`) as the agent streams. Code blocks, diffs and tables are summarised, never read out.
- **Barge-in.** Talking over Dove (stricter VAD thresholds while it speaks, so echo residue does not count) clears the speaker queue, cancels the clause being rendered and drops every queued clause of the old generation.
- **Conductor** (`conductor.rs`). One utterance in, effects out. Order: an armed confirmation phrase, the deterministic `fastpath` grammar (whole-utterance commands only: "stop", "approve", "what's happening", "take me back", "end call"…), the `rules` router ("Claude, how's the auth fix?", "tell Codex to stop", "start a Gemini task on … in a new worktree"), an optional command router (an installed agent's tool-less, read-only run on its cheapest model via `Engine::complete_restricted`, or an OpenAI-compatible endpoint; answers validated against the strict `intent` schema), else the words go to the current thread as a message. Status answers come from the `Snapshot` (status, current activity, todos, `git diff --shortstat`, pending requests, last error), never from asking the agent or guessing.
- **Approvals.** Voice answers the same `PermissionRequest` a click answers and never widens a thread's mode. `safety.rs` classifies requests; a destructive one (recursive delete, force push, history rewrite, `curl | sh`, secrets, paths outside the thread's folder) is never approved by voice: the user says a confirmation phrase and then clicks Approve in the chat, and a changed request drops the confirmation. The shell re-checks the classification before acting.
- **Transcript.** Spoken turns are sent like typed ones, so they land in each thread's JSONL transcript in order. `~/.dove/voice/calls.jsonl` adds what the transcript format has no place for: that a turn was spoken, which call it belonged to, Dove's own spoken lines, and call duration. No audio is stored.
- **Memory.** Nothing is loaded until a call starts; engines stay cached for five minutes after a call (a warm restart) and are then dropped. Dictation's cached Parakeet is unloaded when a call starts so the model is never resident twice.
- **Outside Dove.** On Windows the call shortcut is also a system hotkey (`hotkey.rs`: `RegisterHotKey` on its own message-loop thread, re-registered when the binding changes, released when turned off). While a call runs the shell notes the foreground window once a second (`look.rs`); "take a look at this" with no Dove panel open captures that window with `PrintWindow` into `~/.dove/voice/look/` and attaches its path.
- **Remote-ready.** A call's input can be `Input::Stream`: mono samples pushed by someone else. That is how a phone remote can stream its microphone into the desktop conductor, and how `tests/call.rs` drives the real pipeline without a microphone.

Measurements and methodology: [docs/VOICE.md](docs/VOICE.md).

## Principles

- **The engine records, the UI renders.** Session activity is written in one place.
- **One boundary per concern.** Harnesses enter through `spawn`; extra tools enter through the `dove` MCP server; the browser is driven through one control endpoint.
- **Zero idle cost.** No animation loops when nothing changes; the vendored `gpui-base` stops the caret blinking when idle so an idle window renders zero frames.
- **Honest UI.** No fake buttons; disabled actions explain why.

## Credits

Dove adapts code from [Zeron](https://github.com/zeronsh/zeron) (MIT) and uses [zui](https://github.com/zeronsh/zui) and gpui-component. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
