# The launcher: what the others do, and what Keplorayx's does

Alt+Space, from any app. Written 2026-11-05, before and while building
`src/launcher.rs` and `ui/launcher.luau`.

## What the others do

| Launcher | What stands out | Sources |
| --- | --- | --- |
| **Raycast** (macOS, Windows beta) | Extensions store; clipboard history (text, images, links, files; kept for months on Pro); snippets with placeholders (`{clipboard}`, `{date}`, `{uuid}`); quicklinks with arguments; window management (halves, thirds, next display); AI chat and AI commands over many model providers, shareable with a team; MCP | [Raycast review 2026 (dev.to)](https://dev.to/jangwook_kim_e31e7291ad98/raycast-review-2026-the-mac-launcher-that-replaced-4-apps-mcp-setup-guide-3ag2), [PCWorld on Raycast for Windows](https://www.pcworld.com/article/3016518/raycast-is-finally-on-windows-and-it-totally-changed-how-i-use-my-pc.html), [Raycast Windows changelog](https://www.raycast.com/changelog/windows/0-24) |
| **Spotlight** (macOS Tahoe) | Quick Keys (short typed codes for actions, learned from use); actions with parameter fields filled in place; clipboard history with a retention choice and a warning about sensitive data | [MacRumors](https://www.macrumors.com/2025/06/09/apple-supercharges-spotlight-in-macos-tahoe-with-quick-keys-and-more/), [AppleInsider](https://appleinsider.com/inside/macos-tahoe/tips/spotlight-got-a-huge-upgrade-in-macos-tahoe-heres-how-to-use-it), [iThinkDiff](https://www.ithinkdiff.com/spotlight-macos-tahoe-quick-keys-clipboard/) |
| **PowerToys Command Palette** (Windows) | Extension gallery inside its settings; a Dock of pinned commands on the desktop; fallback ranking you reorder; file previews (Peek); extensions by Store or WinGet | [PowerToys on GitHub](https://github.com/microsoft/powertoys), [Neowin on 0.97](https://www.neowin.net/amp/powertoys-097-is-out-with-a-big-update-for-one-of-its-best-utilities-and-a-new-mouse-tool/), [Windows Central on 0.98](https://www.windowscentral.com/software-apps/my-favorite-free-windows-11-app-just-got-another-great-update-whats-new-in-powertoys-v0-98) |
| **Alfred, Flow Launcher, Wox, Ulauncher, Kunkun** | Workflows and plugins as the core; Everything-backed file search (Flow); shell commands; web searches by keyword; browser bookmarks | known from their own docs; not re-checked tonight |

## What they share, and so ours has

- **One field, keyboard first.** Arrows, Enter, a second action on
  Ctrl+Enter, Tab between scopes, Escape back out. Ours is Zeron's command
  palette (vendored), which already works this way.
- **Instant answers above the search:** a sum, a unit, a conversion,
  before any host search returns. Ours: `ui/kx/calc.luau` (a parser of its
  own, never `load`).
- **Apps, windows, files, clipboard** in one list, best first. Ours finds
  Start menu apps (packaged ones too, via `Get-StartApps`), open windows
  (EnumWindows), files under Desktop, Documents, Downloads, Pictures and
  your projects, and what you copied this run.
- **Quick links and snippets.** Ours: a site's search as a page (type,
  Enter); snippets kept from what you copied, Enter copies one back.
- **System commands that lose nothing:** lock, Task Manager, screenshot,
  Windows settings pages. No shut down or restart from a stray Enter.
- **AI in place.** Raycast's AI chat, Spotlight's actions. Ours asks a
  Keplorayx thread with the composer's agent and model, its answer
  streaming in the launcher, with Open in Keplorayx to carry on.

## What Keplorayx's does its own way

- **Your agents, not one model.** "Ask Keplorayx" runs the harness you
  chose (Claude Code, Codex, Gemini, OpenCode) on your project, so the
  answer can read your code.
- **Clipboard history is never written down.** It's kept in memory for the
  run only. Spotlight's warning about sensitive data is why.
- **Threads and projects** are first-class rows: Enter opens the main
  window on that thread.

## Not yet

- Window management (halves, thirds), emoji, contacts, calendar, file
  previews, a Dock. The calendar and music arrive with the widgets.
- Plugins adding providers to the launcher: the plugin system's next step.
- Quick Keys learned from use; ranking by frecency beyond the palette's own
  recent list.
- Images in clipboard history.
