# Keplorayx on zgx

Keplorayx's interface rebuilt on [zgx](https://github.com/zeronsh/zgx). The
interface is Luau (`ui/`), and it reloads as it's saved without losing
state. The Rust host (`src/`) makes the windows, runs Dove's engine
(`crates/dove-core`) in the same process, and answers the scripts' calls.

This branch (`zgx/main`, in its own clone) replaces `apps/dove`, the GPUI app.
`apps/dove` stays in the tree, out of the build, as the reference while its
UI-free modules move into `crates/dove-app`.

## Running it

```sh
cargo run -p keplorayx                 # the main window
cargo run -p keplorayx -- gallery      # the design system's workbench
cargo run -p keplorayx -- --inspect    # devtools for `zgx inspect` and its MCP server
```

- **Builds** go to `C:/dvw/target-kx-zgx` (`.cargo/config.toml`). They never
  touch the GPUI checkout's `target-ui-v4`.
- **Data** goes to `%LOCALAPPDATA%\keplorayx-zgx-dev` unless `DOVE_HOME` says
  otherwise, so a development build never meets a running Dove's threads or
  settings.
- **Logs:** `KX_LOG=debug` turns on the engine's logging.
- **Frozen scripts:** `KX_UI=<dir>` runs a frozen copy of `ui/`, so a test rig
  isn't reloaded by other saves.

To try a thread without paying for an agent, seed a home with the fake agent
from `scripts/perf/` (`seed_home.py`) and point `DOVE_HOME` at it.

## Layout

```
src/
  main.rs          windows, the engine, the script host
  host.rs          the engine for the scripts: kx.threads, kx.thread, kx.entries, kx.draft; kx.open, kx.send…
  transcript.rs    AgentEvents folded into entries and a draft (no zgx; unit tested)
  transparency.rs  Windows' Transparency effects, watched (ported from Zeron's zgx app)
ui/
  app.luau         the main window
  gallery.luau     every kx component, dark and light
  kx/              the Orbit design system: tokens, themes, components
    store.luau     the host's state and calls, in one place (a fixture stands in headless)
  screens/         one folder for each screen
zgx-asks/          one folder for each gap in zgx, each with a repro and the workaround used here
```

## Rules

- **Values live in the theme.** Screens read tokens (`$space.3`,
  `$text.secondary`). `ui.theme.strict("warn")` flags anything hard-coded.
- **Data changes in place.** Lists are keyed. A streamed word writes
  `kx.draft` alone, and the host republishes `kx.entries` only when an entry
  comes, goes or changes.
- **Nothing blocks the window's thread.** The store, git and harness launches
  run on threads, and their answers come back through async actions.
- **No polling.** Session events are awaited (`recv_async`), and the
  registry is watched (`RegNotifyChangeKeyValue`).
- **Port mechanisms from Zeron, never its look.** Credit anything ported in
  `THIRD_PARTY_NOTICES.md`.
