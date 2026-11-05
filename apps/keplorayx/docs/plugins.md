# Writing a Keplorayx plugin

A plugin is a folder in `<data>/plugins/<id>/` (`%LOCALAPPDATA%\keplorayx-zgx-dev\plugins` unless `DOVE_HOME` says otherwise). It holds `kx-plugin.json` and `init.luau`. zgx runs it in Keplorayx's main window in a sandbox of its own: it reaches only what its manifest asks for and you allowed. The model is Zeron's (its `docs/plugins.md`, MIT).

Agents can write plugins for you through Keplorayx's own tools (`create_plugin`, `write_plugin_file`, `reload_plugins`). Whoever writes it, a plugin runs only once you allow it in **Settings › Plugins**.

## Manifest

```json
{
  "id": "focus-timer",
  "name": "Focus timer",
  "version": "1.0.0",
  "description": "A timer under the sidebar's actions.",
  "entry": "init.luau",
  "permissions": ["ui.add", "sessions.read"]
}
```

- `id`: 2 to 40 lowercase letters, digits and dashes. `kx`, `keplorayx`, `app` and `templates` are taken.
- `permissions`: only what you use. Allowing a plugin allows exactly these. An update that asks for more stops it until you allow it again.
- `mention` (optional): a word the composer offers after `@` (`"computer"` gives `@computer`).
- `mcp` (optional, needs `tools`): a program that speaks MCP over stdio, `{ "command", "args", "env", "instructions" }`. Keplorayx starts it for each agent and relays its tools under the plugin's id. `instructions` are added to what agents are told.
- `requires` (optional): what to install first, as Settings says it.

## Permissions

| permission | lets the plugin |
|---|---|
| `ui.add` | add before or after Keplorayx's elements (`ui.contribute`) |
| `ui.change` | replace Keplorayx's elements |
| `sessions.read` | read `kx.threads` and `kx.thread` (titles, status, agents) |
| `messages.read` | read `kx.entries`, the open thread's messages |
| `navigate` | open threads (`kx.open`) |
| `compose` | put text in the composer (you still send it) |
| `network` | `ui.fetch` and remote pictures |
| `clipboard` | read and write the clipboard |
| `files` | open files and folders you choose |
| `web` | web pages |
| `windows` | windows of its own |
| `terminals` | terminals |
| `tools` | an MCP program of its own for agents (`mcp`) |

Every plugin can read `kx.now`, the time to the minute.

## Code

```lua
-- init.luau: a line under the sidebar.
ui.contribute { target = "#sidebar", after = ui.text { "Hello from a plugin", text_size = 12 } }
```

`after`, `before` and `replace` each take a node. Target an element by its id (`#sidebar`). An error is reported as the plugin's own, in the window and through `reload_plugins`, and the rest of Keplorayx carries on. A saved change reloads the plugin.

## Helpers

Every plugin Keplorayx makes gets `kx.luau` beside its `init.luau` (from `apps/keplorayx/plugins-sdk/`, after Zeron's `zeron.luau`):

```lua
local kx = require("./kx")("focus-timer")
kx.place { at = "#sidebar", after = ui.text { "Hello", text_size = 12 } }
kx.toast("Focus timer", "Started")
local runs = kx.store("runs", 0)          -- kept between runs
kx.every(60, function() … end)            -- 30 s at least; returns a stop
kx.open(thread_id)                        -- navigate
kx.compose("text")                        -- compose
kx.threads(), kx.thread()                 -- sessions.read
```

## Settings

Declare them in the manifest and Keplorayx draws them, under the plugin's name in **Settings › Plugins**, with its own rows. You never draw them.

```json
"settings": [
  { "id": "show", "kind": "switch", "label": "Show the timer", "description": "Under the sidebar.", "default": true },
  { "id": "length", "kind": "number", "label": "Length", "min": 5, "max": 90, "unit": "min", "default": 25 },
  { "id": "sound", "kind": "choice", "style": "segmented", "label": "Sound",
    "options": [ { "value": "bell", "label": "Bell" }, { "value": "none", "label": "None" } ], "default": "bell" },
  { "id": "label", "kind": "text", "label": "Label", "placeholder": "Focus", "default": "Focus" }
]
```

| kind | value | extra fields |
|---|---|---|
| `switch` | `true` / `false` | |
| `choice` | one option's `value` | `options` `[{ value, label }]`, `style` `"select"` or `"segmented"` |
| `text` | text, 2000 characters at most | `placeholder` |
| `number` | kept within `min` and `max` | `min`, `max`, `step`, `unit` |

Read them (no permission):

```lua
local length = kx.setting("length")       -- :get() follows it, :peek() doesn't
kx.settings()                             -- every value by id
kx.on_setting(function(id, value, was) … end)
```

A setting never set reads its `default`; a value that no longer fits reads the default again. What you set goes when the plugin is removed. Every plugin can read every plugin's settings, so a plugin should never ask for a secret there.

## The catalog

Plugins Keplorayx offers sit in `apps/keplorayx/plugins/`. **Install** copies one into your plugins folder, and it still waits for **Allow**. Computer use is the first one.
