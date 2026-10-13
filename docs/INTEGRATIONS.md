# Integrations

Settings → Integrations holds everything that gives Dove's agents reach beyond the project folder. All of it is off until you turn it on, and all of it is bring-your-own: no Dove account, no hosted relay.

Changes apply to agents started after the change. Every integration rides on Dove's own MCP server (`dove --mcp-browser`), which Dove injects into every harness it starts (Claude Code, Codex, OpenCode, Cursor, Pi and ACP agents), so there is nothing to configure per harness. That server is on while **Agent browser** is on.

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

## Composio

[Composio](https://composio.dev) gives agents GitHub, Gmail, Slack, Notion and 1000+ other apps. Paste your own key and turn it on.

- **Connect key** (from the AI Clients setup in the Composio dashboard): Dove uses Composio Connect at `https://connect.composio.dev/mcp` with the key in the `x-consumer-api-key` header. Agents search for tools and authorize apps on demand; the toolkit picks don't apply.
- **Project key** (`ak_…`, header `x-api-key`) or **user key** (`uak_…`, header `x-user-api-key`): when an agent starts, Dove creates a session with `POST https://backend.composio.dev/api/v3.1/tool_router/session`, limited to the toolkits you picked (`{"toolkits": {"enable": [...]}}`; none picked means unrestricted), and connects to the MCP URL and headers that session returns. Dove refuses to send the key to a host outside `composio.dev`.

Tools appear to agents as `composio__<TOOL>`. The key is stored in `~/.dove/integrations.json` (mode 0600) and never placed in a command line or harness config.

## Paper

[Paper](https://paper.design) Desktop runs a local MCP server at `http://127.0.0.1:29979/mcp` while it is open. Turn on **Paper design files** and agents get `paper__*` tools to read and edit the open file, along with an instruction to check the design before building UI and to write back when asked to iterate. Settings shows whether Paper Desktop is running.

## Computer use

Dove uses the open-source [`cua-driver`](https://github.com/trycua/cua) (MIT) running on your machine, the same driver Cosmos pins (`0.34.0`). There is no Cua Cloud and no shell or AppleScript fallback: without the driver there is no computer use.

Install it from [cua.ai/docs/cua-driver](https://cua.ai/docs/cua-driver). Dove finds `cua-driver` on `PATH`, in `~/.local/bin`, `~/.cua/bin`, `~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin` or inside `/Applications/CuaDriver.app`. Settings shows the version and path. **Check** runs the driver's own `health_report` and shows its permission state. On macOS, grant Accessibility and Screen Recording to CuaDriver.app (see the driver's README on process identity).

When it's on, agents get the driver's tools as `computer__*` (`cua-driver mcp` over stdio).

## Status

Tested on this build: the extension loads unpacked in Chrome, pairs, and an agent drove a real tab through `chrome_*` (see the PR). Composio, Paper and cua-driver are wired and their status shows in Settings, but they were not exercised end to end here: there was no Composio key, Paper Desktop or cua-driver install on the test machine.
