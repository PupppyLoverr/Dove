# Integrations

Settings → Plugins (MCP servers, GitHub CLI, ego lite, Dove for Chrome, Paper, Dove Computer Use) and Settings → Plugins → Apps (Composio) hold everything that gives Dove's agents reach beyond the project folder. All of it is off until you turn it on, and all of it is bring-your-own: no Dove account, no hosted relay.

Changes apply to agents started after the change. Every integration rides on Dove's own MCP server (`dove --mcp-browser`), which Dove injects into every harness it starts (Claude Code, Codex, OpenCode, Cursor, Pi and ACP agents), so there is nothing to configure per harness. That server is on while **Agent browser** is on, or while any integration below is on.

In terminal mode Claude Code (`--mcp-config`), Codex (`-c mcp_servers.dove.*` overrides) and OpenCode (`OPENCODE_CONFIG_CONTENT`, merged over your own config and any inline config already set) get the same server.

## Dove for Chrome

A Manifest V3 extension (`apps/extension/`) that lets agents work in your own Chromium browser (Chrome, Edge, Brave, Arc) with your logins.

**Install (unpacked):** open `chrome://extensions`, turn on Developer mode, click **Load unpacked** and choose `apps/extension`. Pin it and click the Dove icon to open the side panel.

**Pair:** in Dove open Settings → Plugins → Dove for Chrome → Pairing code → **Copy**, paste it into the side panel and click **Pair**. The panel shows *Connected* while Dove runs. You pair once: Dove keeps the port and token in `~/.dove/extension.json` (mode 0600) and reuses them on every launch. The extension source contains no secrets; the code lives only in the browser profile's extension storage.

**How it connects:** Dove listens on `127.0.0.1` (a random port picked on first run) for a WebSocket at `/extension`. It only upgrades connections whose `Origin` is a `chrome-extension://` URL, and the first message has to carry the pairing token. Agents call `chrome_*` tools on Dove's MCP server; Dove forwards each call over that socket and returns the result.

**Tools:** `chrome_tabs`, `chrome_tab_open`, `chrome_tab_close`, `chrome_tab_activate`, `chrome_navigate`, `chrome_snapshot` (an accessibility-style outline whose elements carry refs like `[ref=e12]`), `chrome_click`, `chrome_type`, `chrome_press`, `chrome_scroll`, `chrome_hover`, `chrome_select`, `chrome_screenshot` (`chrome.tabs.captureVisibleTab`), `chrome_read`, `chrome_wait`, `chrome_release`. Clicks, hovers, typing and key presses are real input events sent through `chrome.debugger` (CDP), so Chrome shows its "started debugging this browser" bar while an agent acts.

**You stay in charge:**

- **Per-site allow.** The first time an agent touches a site, the page asks *Allow Dove on this site?* Nothing is read or clicked until you allow it. The side panel's switch turns a site on or off at any time.
- **Indicator.** A tab Dove is driving shows a hairline frame and a *Dove is controlling this tab* pill with **Stop**.
- **Stop.** Stop (in the page or the panel) pauses Dove in this browser, detaches the debugger and removes the indicator. Agents get an error until you click **Resume**.
- **Activity.** The side panel lists every action with its arguments and outcome.

`chrome://` pages, the Web Store and other extension pages cannot be scripted by any extension, so agents can't act there.

The existing `browser_*` tools are unchanged and still drive Dove's in-app browser.

## MCP servers

Any MCP server, added once, reaches every harness. Its tools appear to agents as `<id>__<tool>` on Dove's server (names longer than Claude's and Codex's 64-character limit are shortened with a stable hash; Dove maps them back).

**Catalog.** One-click entries for Figma, GitHub, Linear, Notion, Sentry, Supabase, Stripe, Vercel, Atlassian, Cloudflare, Neon, PostHog, Context7, Exa, Firecrawl, Playwright, Chrome DevTools and Supermemory (`apps/dove/src/integrations_catalog.rs`). Remote servers connect over streamable HTTP; Playwright and Chrome DevTools run locally with `npx` (Node.js required).

**Sign-in.** Servers that support MCP authorization use Dove's own OAuth (`apps/dove/src/mcp_oauth.rs`): Dove reads the server's Protected Resource Metadata (RFC 9728) and its authorization server's metadata (RFC 8414 / OpenID discovery), registers itself as a public client (RFC 7591), and opens your browser for an authorization-code + PKCE (S256) sign-in that returns to a one-shot `127.0.0.1` port. Tokens are stored in `~/.dove/mcp_auth.json` (mode 0600 on macOS and Linux) and refreshed on expiry or a 401. You sign in once, in Dove; no harness runs its own OAuth. Servers that take a key instead get it as a header (`Authorization: Bearer …` or the server's own header).

**Custom servers.** **Add server** takes a local command (command, arguments, `KEY=value` environment lines), a remote URL (headers, optional browser sign-in), or a pasted JSON block in Claude Code / Cursor / Gemini (`mcpServers`), OpenCode (`mcp`) or VS Code (`servers`) shape. **Test** starts the server, lists its tools and reports the count. On Windows, `npx`/`.cmd` shims are started through `cmd.exe` without a console window.

Read-only Supermemory tools (`search_memory`, `get_profile`, `list_*`) are approved automatically; every other proxied tool goes through the harness's normal permission prompt.

## ego lite

[ego lite](https://lite.ego.app/) is a Chromium browser built for agents: each task gets its own space inside your own browser profile (your sign-ins), which you can watch and take over. Dove detects its `ego-browser` command and ships the **dove-browser** skill (`skills/dove-browser/`), installed for every harness next to the office skills. The skill loads ego's version-matched guide (`ego-browser skill`) and adds Dove's playbook: one task space per goal, observe-act-verify rounds, ref-first selectors, waits, compact extraction, PowerShell-safe script files, safety rules (no credentials, ask before irreversible actions, hand off at sign-in walls and CAPTCHAs) and per-site learnings saved to memory. Without ego lite the skill falls back to Dove's `browser_*` tools. **Agents browse with** picks the agents' browser: **ego lite** (Dove's own `browser_*` tools are hidden, so agents always use ego), **Dove browser** (agents are not pointed at ego lite) or **Both** (the default: ego first, Dove's browser as a fallback). Dove never downloads ego lite; **Get ego lite** opens its site.

**Run without asking** (off by default, saved as `ego_allow` in `integrations.json`, hidden and ignored while agents browse with Dove's browser) lets agents run `ego-browser` without a permission prompt in sessions started afterwards. Dove never matches shell text itself; each harness gets its own allow rule and parses the command on its own terms, so `ego-browser x; rm -rf ~` still asks:

- **Claude Code**, chat and terminal: `--allowedTools Bash(ego-browser:*)` (skipped in Bypass, which already allows everything). Claude splits chained commands before matching.
- **Codex** keeps asking. Its command rules (`prefix_rule` in `.rules` files) load only from `~/.codex/rules` and other config folders, with no launch flag, and Dove does not edit your Codex config.
- **OpenCode** is left alone. Its chat driver already answers each permission ask, and its default bash rule is allow. A `permission.bash` rule in `OPENCODE_CONFIG_CONTENT` would be deep-merged over your files and replace a string `"ask"` policy, loosening it.
- Other harnesses (Cursor, Pi, ACP agents) keep their normal prompts.

## Composio (Settings → Plugins → Apps)

[Composio](https://composio.dev) gives agents GitHub, Gmail, Slack, Notion and 1000+ other apps. Sign in with your Composio account, or paste your own key, and turn it on.

- **Composio account**: Dove signs in to Composio Connect (`https://connect.composio.dev/mcp`) with the MCP OAuth flow above. Agents get Composio's search, link and execute tools, so any app you link in Composio is usable.

- **Connect key** (from the AI Clients setup in the Composio dashboard): Dove uses Composio Connect at `https://connect.composio.dev/mcp` with the key in the `x-consumer-api-key` header. Agents search for tools and authorize apps on demand; the toolkit picks don't apply.
- **Project key** (`ak_…`, header `x-api-key`) or **user key** (`uak_…`, header `x-user-api-key`): when an agent starts, Dove creates a session with `POST https://backend.composio.dev/api/v3.1/tool_router/session`, limited to the toolkits you picked (`{"toolkits": {"enable": [...]}}`; none picked means unrestricted), and connects to the MCP URL and headers that session returns. Dove refuses to send the key to a host outside `composio.dev`.

Tools appear to agents as `composio__<TOOL>`. The key is stored in `~/.dove/integrations.json` (mode 0600) and never placed in a command line or harness config.

## Paper

[Paper](https://paper.design) Desktop installs the Paper CLI to `~/.paper/bin` the first time it opens. When that CLI is there, Dove runs `paper mcp`, its stdio relay, which connects even while Desktop is closed (reads and writes reach the open file once Desktop runs). Without it, Dove falls back to Desktop's legacy HTTP server at `http://127.0.0.1:29979/mcp`, which answers only while a file is open. Turn on **Paper** and agents get `paper__*` tools to read and edit the open file, along with an instruction to check the design before building UI and to write back when asked to iterate. Settings shows whether the CLI was found or Desktop is running.

## Dove Computer Use

Computer use is a plugin you install from the **Dove Computer Use** plate. It runs the stock, open-source [`cua-driver`](https://github.com/trycua/cua) (MIT, signed by Cua AI), which works in the background through each window's accessibility tree (UIA on Windows, AX on macOS, AT-SPI on Linux). Agents never see the driver's own tools: Dove wraps it in **one `computer` tool** (`crates/dove-computer`, a Rust port of Hermes Agent's `computer_use` layer). There is no Cua Cloud and no shell or AppleScript fallback: without the driver there is no computer use.

- **Install** downloads this platform's archive of the pinned cua-driver release from trycua's GitHub release (`trycua/cua`, tag `cua-driver-rs-v<version>`). Dove trusts only the manifest compiled into it (`apps/dove/assets/computer-use-release.json`, `manifest_version` 1, platform keys `windows-x86_64`, `windows-arm64`, `macos-universal`, `linux-x86_64`, `linux-arm64`, written from the release's archives by `scripts/computer-use/update_manifest.py`): the download must have exactly the listed size and SHA-256, and the unpacked archive must hold exactly the listed files under its one `root/` folder, each with its hash, with no links and no paths outside it. On Windows the driver is `cua-driver.exe` with its `cua-driver-uia.exe` sidecar; on macOS it is the signed `CuaDriver.app` (its TCC identity is `com.trycua.driver`). A release whose `min_dove_version` is newer than this Dove is refused. It lands in `~/.dove/plugins/computer-use/<version>/`, and before every launch the hub re-hashes the driver and its sidecar against the install record and refuses to start a changed file. Nothing is downloaded until you click Install.
- **Update** appears when the pinned version is newer than the installed one and replaces it, keeping the driver's state; **Uninstall** removes `plugins/computer-use/` (versions and state). A weekly workflow (`.github/workflows/cua-driver-pin.yml`) opens a pull request when trycua publishes a new `cua-driver-rs-v*` release, with the regenerated manifest.
- **Status** shows the driver's own `health_report` (`ok`, `degraded` or `failed`, with the first failing check and its fix). It runs when the plate opens and on **Check again**.
- **Permissions** opens a walkthrough for this OS, shown once after the first install: macOS Accessibility and Screen Recording for CuaDriver (buttons open those System Settings panes), Linux AT-SPI (a button runs `gsettings set org.gnome.desktop.interface toolkit-accessibility true`) and the Wayland portal prompt, Windows' integrity-level rule (the driver runs as you, so it cannot control apps started as administrator).
- **Approvals**: *Ask per app* (default) shows a card above the thread's input the first time its agent wants to type or click in an app (Deny, Allow once, Allow in this thread); *Background without asking* skips that card. Either way, bringing a window to the front asks separately, and a control whose label sends, buys, deletes, confirms or signs asks every time. Unanswered cards decline after 5 minutes; with no Dove window (a headless run) input is refused, never auto-approved.
- **Never control**: names (`1Password, KeePass`), or `signer:<publisher or Team ID>`, `bundle:<bundle id>` and `path:<prefix>` rules. Signer rules follow the verified code signature (Windows Authenticode, macOS Developer ID), so renaming an executable does not get around them. A denied app is refused before anything is read from it, and its windows are left out of `list_windows`.
- **Local grounder**: NEXI-CUA-1 (6.4 MB, `crates/dove-grounder/assets/nexi-cua-1.onnx`) is built into Dove; turning it off leaves `act` and intent steps to the deterministic resolver (exact names, ordinals, key chords).
- **Raw driver tools** (off): agents get cua-driver's own `computer__*` tools instead of `computer`, without approval cards or the take-over fence. For debugging the driver only.
- A `cua-driver` you installed yourself (on `PATH`, `~/.local/bin`, `~/.cua/bin`, `~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin` or `/Applications/CuaDriver.app`) is used when the plugin is not installed, as a developer override.

When it's on, the hub starts the driver (`cua-driver mcp` over stdio) with `CUA_DRIVER_RS_HOME=~/.dove/plugins/computer-use/state` (never shared with a cua-driver you run yourself), `CUA_DRIVER_PERMISSION_MODE=standard`, `CUA_DRIVER_RS_TELEMETRY_ENABLED=0`, `DO_NOT_TRACK=1` and `CUA_DRIVER_RS_UPDATE_CHECK=0`. Agents get the `computer` tool (one ~6 KB schema instead of the driver's 66 tools), and the hub's instructions point them to the bundled **dove-computer** skill.

**The `computer` tool.** `capture` returns numbered rows (`#7 Button "Save" @ (x, y, w, h)`), tree-first: a screenshot comes only when the tree is sparse (then OS-native OCR text — Windows.Media.Ocr, Apple Vision, or `tesseract` on Linux — is added as clickable rows) or when asked. `since="latest"` returns only what changed. Rows are backed by the driver's per-snapshot element tokens, so an index from an older capture is refused rather than re-pointed. Every action result carries a `verdict` (`done`, `verify_fresh_state`, `recapture`, `escalate` with `recommended` px / foreground / set_value). `run` sends a batch of steps as one `run_steps` call; `act` and `{intent}` steps are grounded on the device; `verify` checks text, roles, names and absence; `save_replay` / `replay` keep a verified run with role-and-name targets. Hard blocks refuse lock/log-out and trash shortcuts and dangerous shell text; typing into password, PIN and payment fields is refused (the tree's roles and labels, plus UI Automation's `IsPassword` on Windows).

**Take over.** While an agent uses an app, its thread shows *Using <app> in the background* with **Take over**; on Windows, Ctrl+Alt+Shift+S does the same from anywhere. While you have control every computer action is refused, and a result produced across a take-over is discarded.

**Break-glass driver.** `PupppyLoverr/NEXI-Models-Drivers` keeps `dove-driver`, a fork of cua-driver, dormant. Its `scripts/dove/dev-install.py --dove-home <DOVE_HOME>` packages a local build into `plugins/computer-use/<version>-dev/`, which Dove finds like a release (ranked below the same released version) and re-hashes before every launch. Use it only when a blocking fix has not reached a cua-driver release.

For QA against a fake release, debug builds read `DOVE_COMPUTER_USE_MANIFEST` (a manifest file) and `DOVE_COMPUTER_USE_RELEASE_BASE` (a URL that serves the assets). Release builds ignore both.

## Status

Tested on this build: the extension loads unpacked in Chrome, pairs, and an agent drove a real tab through `chrome_*` (see the PR). Composio and Paper are wired and their status shows in Settings, but they were not exercised end to end here. Dove Computer Use's `computer` tool is covered by unit and dispatcher tests against a scripted driver (`cargo test -p dove-computer`); its live run against the real, stock cua-driver 0.38.0 (`DOVE_CUA_DRIVER=<cua-driver.exe> cargo test -p dove-computer --test live -- --ignored --test-threads=1`; it drives real windows, so run it on purpose) passes on Windows 11: background type, click and verify with the foreground window and pointer unchanged, the password field refused by UI Automation, a `since` re-read, a `run` batch, on-device `act`, Windows OCR of pixel-only text, and no remote connections from the driver. macOS and Linux have not been run on real machines yet.

## Memory (Supermemory)

Settings › Plugins › Memory signs in to Supermemory (the same catalog server and OAuth token the hub uses) and turns Dove itself into every harness's memory layer (`crates/dove-core/src/memory.rs`, backend in `apps/dove/src/supermemory.rs`):

- **Two surfaces, one sign-in.** Supermemory's MCP server serves extracted memories, profile, listing, forgetting and `who_am_i`; its REST API (`/v3/documents`, `/v3/search`) takes saves with real metadata and an idempotent `customId`, and searches documents. Both accept the OAuth token Dove signed in with. Supermemory extracts memories from documents later on its side (minutes or more), so recall searches documents too and a turn captured a moment ago is found on the next prompt. Document hits under a 0.6 hybrid score are dropped.
- **Recall.** Every prompt Dove hands an agent (first turn, resumed turn, live send, automations, teammates) passes through the session recorder, which searches memory with it (plus the profile on a session's first turn) across the project, user and teammate scopes, drops what the session already saw, and prepends a `<memory source="dove-recall">` block of about 1.5k tokens. It waits at most 4 s. The chat shows a "Recalled N memories" chip instead of the block.
- **Local index.** Every memory Dove saves (already redacted) is also appended to `$DOVE_HOME/memory/local.jsonl`. Recall searches it first, instantly and offline, then merges whatever Supermemory answers within 3.5 s (MCP calls wait at most 2.5 s). Supermemory search latency varied from 0.1 s to 27 s in testing, so without the index a slow service meant no memories at all. Forgetting a memory in Settings removes it from the index too.
- **Standing rules.** Turns where the user states a rule or preference ("always", "never", "my preference") or corrects earlier knowledge ("no, that changed from A to B") are saved as `kind: rule`, and every session's first prompt gets all rules and corrections for its scopes, whatever it is about. Recalled items carry their save time, and the newer of two disagreeing memories wins.
- **Capture.** On each turn end the prompt, final answer and edited files are redacted (`[REDACTED]` for keys, tokens, passwords, private keys, URL credentials, `<private>` spans) and saved with `{source: "dove", harness, project, task}`. Paths matching the never-send globs are left out. Failed saves queue in `$DOVE_HOME/memory/outbox.jsonl` and retry on the next save, sign-in or launch.
- **Corrections.** A prompt that corrects or re-asks the last turn, and a denied permission, become high-priority `feedback` memories.
- **Scopes.** The project container is `repo_<name>__<sha256(normalized origin remote)[..16]>` (resolved path without a remote), honouring `.claude/.supermemory-claude/config.json` `repoContainerTag` and `SUPERMEMORY_REPO_TAG`, exactly as Supermemory's Claude/Codex/OpenCode plugins derive it, so memories are shared with them. The user scope is the account's default space; teammates get `team_<id>`.
- **Warm-up and consolidation.** "Warm up" imports past Dove threads, project READMEs / ARCHITECTURE / CLAUDE.md / AGENTS.md and recent merged PRs. A weekly consolidation lists each scope, finds duplicates (forgotten only when you click) and drafts `SKILL.md` files for repeated command workflows in `$DOVE_HOME/memory/suggested-skills/`. Each draft has **Review draft** (opens it) and **Install**, which installs it for every harness like a GitHub skill (source "You" in Settings › Plugins › Skills).
- **Browse.** Settings › Plugins › Memory searches any scope, ranked or as a **Timeline** grouped by day; **Graph** opens your memory graph in Supermemory's app; **Export** writes a JSON file.

End-to-end check against a signed-in account (scratch home with `mcp_auth.json`; strip any `CLAUDE*`/`ANTHROPIC*` variables when running from inside an agent):

```
DOVE_DATA=<home> cargo test -p dove supermemory::e2e::pipeline_against_account -- --ignored --nocapture
```

`supermemory::e2e::claude_teaches_codex` runs the same flow with real Claude Code and Codex sessions.
