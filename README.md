# Dove

Dove is a native desktop Agentic Development Environment. One window runs many coding agents at once. Each task gets its own git worktree, a live transcript, a diff review, and a cost ledger that stays in one place no matter which agent did the work.

The UI is GPU-rendered with [GPUI](https://github.com/zeronsh/zui). There is no Electron shell and no embedded web app for the product itself.

## Why it's different

- **One window, many agents.** Claude Code, Codex, Cursor, OpenCode, Devin, Gemini, Copilot, and the other harnesses below share the same projects, threads, and review tools. You do not switch apps to switch agents.
- **Worktrees by default.** A task can run on its own branch so two agents can edit the same repo without overwriting each other.
- **Chat and the agent's own terminal are the same session.** Chat is the rich view (permissions, steering, usage). Terminal hands that session to the agent's TUI and mirrors it back, then chat resumes it.
- **The side panel is part of the work.** Diffs, git history, a file viewer, an in-app browser, a terminal, and a side chat stay attached to the thread.
- **Any harness can drive the in-app browser.** Tool calls such as `browser_navigate`, `browser_click`, `browser_type`, `browser_press`, `browser_scroll`, and `browser_snapshot` open the page and act on it the way a person would.
- **The transcript is a document, not a log dump.** Answers render markdown, tables, Mermaid diagrams, math, and images (including TIFF). Tool calls fold. An agent can also emit an interactive card (` ```ui ` / ` ```widget `) that the user can change; the choice goes back as a normal message so every harness can continue.

## Use cases

- Start a feature in chat, watch the diff appear in the side panel, and open a file from a `path:line` link without leaving the thread.
- Run a second agent on the same branch as a side chat while the parent thread keeps going.
- Hand a session to the agent's terminal UI when you want its native controls, then come back to chat on the same session.
- Let an agent open a local dev server in the in-app browser and click through the page.
- Preview images, markdown, and commits inside the panel, then ask the agent to edit the file you are looking at.
- Keep a day's usage in one ledger instead of reconstructing it from each agent's own UI.

## Architecture

```text
apps/dove            GPUI window: sidebar, threads, composer, transcript, side panel
crates/dove-core     Engine. Owns the runtime, live sessions, and the task lifecycle
crates/dove-harness  Agent adapters. Normalizes each CLI into one event stream
crates/dove-proto    Shared types: events, tools, permissions, usage
crates/dove-store    SQLite: projects, tasks, transcripts, settings, usage
crates/dove-git      Worktrees, status, diffs, history
crates/dove-term     Terminal emulator and PTY sessions
```

The UI never writes session activity itself. It watches `LiveSession` events and sends commands. The engine appends each event to the transcript, updates task status and usage, then forwards it.

Harnesses enter through one boundary. Claude Code uses its stream-json protocol. Codex uses app-server JSON-RPC. Most other agents speak ACP, with Zeron's adapters bridged in where they already know that agent's catalog and session format. Every adapter emits the same `AgentEvent`s, so the transcript, the plan, the diff, and the browser driver do not care which CLI produced them.

The side panel is per thread. Browsers and terminals stay alive when you switch surfaces. History is local git. The file viewer highlights source, renders markdown and images, and can save an edit or hand the instruction to the agent. The browser is a native page (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) with GPUI chrome. Agent control goes through the page, not through a separate browser product.

Chat rendering is incremental: finished markdown blocks become their own rows, and only the open tail is re-parsed while tokens arrive. Diagrams, math, and generative cards are blocks in that same stream.

## Supported agents

| Agent | Integration |
| --- | --- |
| Claude Code | stream-json |
| Codex | app-server JSON-RPC |
| OpenCode | ACP |
| Devin | ACP |
| Cursor | ACP |
| Grok Build | ACP |
| Gemini CLI | ACP |
| Qwen Code | ACP |
| Copilot | ACP |
| goose | ACP |
| Factory Droid | ACP |

Antigravity, Pi, Hermes, Amp, Kilo, and Cline are wired through the same adapter path where their CLIs are installed. Model lists refresh from the live catalog when the CLI answers.

## Building from source

Requires Rust stable 1.90 or newer.

```sh
cargo run -p dove            # or: cargo run -p dove -- path/to/repo
```

Linux system dependencies:

```sh
sudo apt-get install -y libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libssl-dev pkg-config
```

## Data and logging

- Data lives in `~/.dove`. Override the location with the `DOVE_HOME` environment variable.
- Verbose logs: set `DOVE_LOG=debug`.
- The first launch on a device plays a short intro (click or press Esc to skip). `DOVE_SKIP_INTRO=1` turns it off; `DOVE_SHOW_INTRO=1` replays it. Its assets and the shareable video are built in [`media/intro`](media/intro/README.md).

## Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| New session | Ctrl/Cmd+N |
| Add project | Ctrl/Cmd+O |
| Toggle sidebar | Ctrl/Cmd+B |
| Toggle side panel | Ctrl/Cmd+\ |
| Toggle terminal | Ctrl+` |
| Back / forward | Alt+Left / Alt+Right |
| Settings | Ctrl/Cmd+, |
| Send message | Enter |
| Insert newline | Shift+Enter |
| Terminal copy / paste | Ctrl+Shift+C / Ctrl+Shift+V |

Third-party notices (Zeron, zui, Solar Icons, Geist) are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
