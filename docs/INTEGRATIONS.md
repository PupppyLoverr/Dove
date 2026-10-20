# Integrations

Settings → Integrations (MCP servers, GitHub CLI, ego lite, Dove for Chrome, Paper, Dove Computer Use) and Settings → Connectors (Composio) hold everything that gives Dove's agents reach beyond the project folder. All of it is off until you turn it on, and all of it is bring-your-own: no Dove account, no hosted relay.

Changes apply to agents started after the change. Every integration rides on Dove's own MCP server (`dove --mcp-browser`), which Dove injects into every harness it starts (Claude Code, Codex, OpenCode, Cursor, Pi and ACP agents), so there is nothing to configure per harness. That server is on while **Agent browser** is on, or while any integration below is on.

In terminal mode Claude Code (`--mcp-config`), Codex (`-c mcp_servers.dove.*` overrides) and OpenCode (`OPENCODE_CONFIG_CONTENT`, merged over your own config and any inline config already set) get the same server.

## Dove for Chrome

A Manifest V3 extension (`apps/extension/`) that lets agents work in your own Chromium browser (Chrome, Edge, Brave, Arc) with your logins.

**Install (unpacked):** open `chrome://extensions`, turn on Developer mode, click **Load unpacked** and choose `apps/extension`. Pin it and click the Dove icon to open the side panel.

**Pair:** in Dove open Settings → Integrations → Pairing code → **Copy**, paste it into the side panel and click **Pair**. The panel shows *Connected* while Dove runs. You pair once: Dove keeps the port and token in `~/.dove/extension.json` (mode 0600) and reuses them on every launch. The extension source contains no secrets; the code lives only in the browser profile's extension storage.

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

## Composio (Settings → Connectors)

[Composio](https://composio.dev) gives agents GitHub, Gmail, Slack, Notion and 1000+ other apps. Sign in with your Composio account, or paste your own key, and turn it on.

- **Composio account**: Dove signs in to Composio Connect (`https://connect.composio.dev/mcp`) with the MCP OAuth flow above. Agents get Composio's search, link and execute tools, so any app you link in Composio is usable.

- **Connect key** (from the AI Clients setup in the Composio dashboard): Dove uses Composio Connect at `https://connect.composio.dev/mcp` with the key in the `x-consumer-api-key` header. Agents search for tools and authorize apps on demand; the toolkit picks don't apply.
- **Project key** (`ak_…`, header `x-api-key`) or **user key** (`uak_…`, header `x-user-api-key`): when an agent starts, Dove creates a session with `POST https://backend.composio.dev/api/v3.1/tool_router/session`, limited to the toolkits you picked (`{"toolkits": {"enable": [...]}}`; none picked means unrestricted), and connects to the MCP URL and headers that session returns. Dove refuses to send the key to a host outside `composio.dev`.

Tools appear to agents as `composio__<TOOL>`. The key is stored in `~/.dove/integrations.json` (mode 0600) and never placed in a command line or harness config.

## Paper

[Paper](https://paper.design) Desktop installs the Paper CLI to `~/.paper/bin` the first time it opens. When that CLI is there, Dove runs `paper mcp`, its stdio relay, which connects even while Desktop is closed (reads and writes reach the open file once Desktop runs). Without it, Dove falls back to Desktop's legacy HTTP server at `http://127.0.0.1:29979/mcp`, which answers only while a file is open. Turn on **Paper** and agents get `paper__*` tools to read and edit the open file, along with an instruction to check the design before building UI and to write back when asked to iterate. Settings shows whether the CLI was found or Desktop is running.

## Dove Computer Use

Computer use is a plugin you install from the **Dove Computer Use** plate: `dove-driver`, Dove's fork of the open-source [`cua-driver`](https://github.com/trycua/cua) (MIT), which works in the background through each window's accessibility tree (UIA on Windows, AX on macOS, AT-SPI on Linux). There is no Cua Cloud and no shell or AppleScript fallback: without the driver there is no computer use.

- **Install** downloads this platform's archive of the pinned dove-driver release from its GitHub releases (`PupppyLoverr/NEXI-Models-Drivers`, tag `dove-driver-v<version>`). Dove trusts only the release manifest compiled into it (`apps/dove/assets/computer-use-release.json`, the release's `dove-driver-manifest.json`, `manifest_version` 1, platform keys `windows-x86_64`, `windows-arm64`, `macos-universal`, `linux-x86_64`, `linux-arm64`): the download must have exactly the listed size and SHA-256, and the unpacked archive must hold exactly the listed files under its one `root/` folder, each with its hash, with no links and no paths outside it. On Windows that is `dove-driver.exe` and its `dove-driver-uia.exe` sidecar. A release whose `min_dove_version` is newer than this Dove is refused. It lands in `~/.dove/plugins/computer-use/<version>/`, and before every launch the hub re-hashes the driver and its sidecar against the install record and refuses to start a changed file. Nothing is downloaded until you click Install. A platform the manifest does not list says so and offers no Install button.
- **Update** appears when the pinned version is newer than the installed one and replaces it, keeping the driver's state; **Uninstall** removes `plugins/computer-use/` (versions, state and grounder), which removes the `computer__*` tools.
- **Status** shows the driver's own `health_report` (`ok`, `degraded` or `failed`, with the first failing check and its fix). It runs when the plate opens and on **Check again**.
- **Permissions** opens a walkthrough for this OS, shown once after the first install: macOS Accessibility and Screen Recording (buttons open those System Settings panes), Linux AT-SPI (a button runs `gsettings set org.gnome.desktop.interface toolkit-accessibility true`) and the Wayland portal prompt, Windows' integrity-level rule (the driver runs as you, so it cannot control apps started as administrator, and its UIA sidecar runs from Dove's per-user folder without UIAccess).
- **Approvals**: *Ask first* (the driver's `standard` mode) or *Bounded* (acts within its capability manifest without asking). Dove never offers `unrestricted`.
- **Local grounder** (optional): downloads the grounder weights (`optional_assets.grounder`, about 3 MB) from the same manifest, size- and SHA-256-checked, into `plugins/computer-use/grounder/`, so `intent` steps and the `act` tool resolve on the device. Turning it off deletes the weights.
- A `dove-driver` or `cua-driver` you installed yourself (on `PATH`, `~/.local/bin`, `~/.cua/bin`, `~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin` or `/Applications/CuaDriver.app`) is used when the plugin is not installed, as a developer override.

When it's on, the hub starts the driver as the `computer` upstream (`dove-driver mcp` over stdio) with `DOVE_DRIVER_HOME=~/.dove/plugins/computer-use/state`, `DOVE_DRIVER_PERMISSION_MODE` from Approvals, `DOVE_TOOL_DISCLOSURE=lazy` (a small tool list; the rest through `describe_tools`) and, with the grounder on, `DOVE_DRIVER_GROUNDER_MODEL`. Agents get its tools as `computer__*`, and the hub's instructions point them to the bundled **dove-computer** skill (installed to every harness by Settings › Skills): `list_windows`, a query-scoped `get_window_state`, `run_steps` with element tokens or `intent` steps, `verify_state` at checkpoints, screenshots only when the tree is empty or pixels are needed.

For QA against a fake release, debug builds read `DOVE_COMPUTER_USE_MANIFEST` (a manifest file) and `DOVE_COMPUTER_USE_RELEASE_BASE` (a URL that serves the assets). Release builds ignore both.

## Status

Tested on this build: the extension loads unpacked in Chrome, pairs, and an agent drove a real tab through `chrome_*` (see the PR). Composio, Paper and the computer-use driver are wired and their status shows in Settings, but they were not exercised end to end here: there was no Composio key, Paper Desktop or cua-driver install on the test machine.
