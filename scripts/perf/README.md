# Dove perf harness

Measures startup, idle and streaming CPU, RAM by process class and keypress latency for 1, 5 and 10 concurrent sessions. It costs no tokens: every session runs the fake agent. It never touches `~/.dove`, and it never takes the foreground unless you pass `-Activate`.

## Pieces

| Path | What it is |
|---|---|
| `fake-agent/` | `dove-fake-agent`, a stand-in agent CLI. It speaks Claude Code stream-json (when its args contain `stream-json`) or ACP (otherwise), and streams synthetic turns: thinking, markdown text, and a tool call every few seconds. Tuned with `FAKE_AGENT_SECS`, `FAKE_AGENT_CPS`, `FAKE_AGENT_CHUNK`, `FAKE_AGENT_TOOL_EVERY` and `FAKE_AGENT_TURNS`. |
| `seed_home.py` | Seeds a scratch `DOVE_HOME`: a git project, `agent_path.*` pointing Claude Code, Gemini, Qwen and Copilot at the fake agent, and N automations that each start one session on the scheduler's first tick. |
| `perf.ps1` | Runs the whole scenario and writes `results.json` + `summary.md`. |
| `apps/dove/src/perf_trace.rs` | Opt-in hooks in Dove: `DOVE_INPUT_TRACE`, `DOVE_STARTUP_TRACE`, `DOVE_NO_ACTIVATE`. Off by default. |

Dove's existing `DOVE_FRAME_STATS` (shell renders per second) and GPUI's `ZED_MEASUREMENTS` (per-frame duration) supply the render and frame-time numbers.

## Run

```powershell
cargo build --release -p dove
cargo build --release --manifest-path scripts/perf/fake-agent/Cargo.toml
powershell -ExecutionPolicy Bypass -File scripts\perf\perf.ps1 -Exe target\release\dove.exe -Sessions 1,5,10
```

One session count takes about 4 minutes: about 60 s to the scheduler tick, a 150 s turn, and idle samples before and after.

## What each number means

- **window_ms**: process start until the window is visible.
- **first_frame_ms**: process start until the first frame is built. This is process creation → `main` (from the OS start time) plus `main` → first root render.
- **idle**: 15 s sampled 20 s after launch, after the launch work and the caret blink have settled. Reports CPU (% of one core) and shell renders/s.
- **streaming**: a 30 s sample while all N sessions stream.
- **memory**: private and working-set MB summed per class. `dove-main`, `dove-helper` (Windows process escrow), `dove-mcp-bridge` (`dove --mcp-browser`), `dove-webview` and `dove-mesh-host` are Dove's own; `harness:*` are the agents, reported separately.
- **input_latency**: posted key presses into the window. For each one, key → next frame start (`DOVE_INPUT_TRACE`) + that frame's duration (`ZED_MEASUREMENTS`). Reported as p50/p95/p99.
- **after_sessions**: the idle sample again once the turns end, to catch timers and renders that outlive the work.

## Caveats

- By default the window is visible but not focused. GPUI caps an inactive window that keeps animating at about 30 fps, so streaming CPU in that mode is lower than in a focused window. `-Activate` measures the focused case, but it holds the foreground for the whole run.
- By default the PATH is minimal (System32 + git), so real agent CLIs are not detected and the startup model-catalog refresh does not run. `-RealPath` includes them; use it to measure that startup cost.
- The scheduler starts sessions in the background; they are not open in a tab. That measures background-session cost, which is the common case with 10 sessions.
