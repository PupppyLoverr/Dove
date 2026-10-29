# Keplorayx vs Claude: UI/UX review

Written 2026-10-29 from screenshots of every Keplorayx page (build `integrate/k22` + the top-bar and composer changes of the same day), compared with Claude's desktop app and claude.ai (chat, Projects, Artifacts, Settings, Claude Code). The K22 icon system stays as it is; nothing here changes icons.

Priority: **P0** fix now (a bug or a confusing first impression), **P1** next pass, **P2** polish.

## What Claude does that we should copy (the short list)

1. **One calm column per page.** Claude's pages are a single centred column (~720 px) with a title, a one-line description and one primary action, top right. Ours mostly do this; the places that don't are listed below.
2. **A greeting, not a placeholder, on the new-chat page.** "Good evening, Ravth" in a large serif, with the composer under it. Our Home leads with the composer's placeholder ("Do anything…") and a stack of cards.
3. **Suggestion chips under the composer** (Write, Learn, Code…), each filling a starter prompt. Ours shows folder and device chips there, which are settings, not starts.
4. **Everything secondary behind one `+`.** Claude's `+` holds attach, connectors, tools and modes. Ours spreads attach, mode, model, call and mic across the bar.
5. **Hover-revealed actions.** Recents show `⋯` only on hover; message actions (copy, retry) only on the last reply or on hover. We do this for replies already; do it for sidebar rows too.
6. **Settings as a page with plain sections**, not cards inside cards: a heading, rows separated by hairlines, the control on the right.
7. **Quiet empty states** with one line and one button. Ours are good; keep them consistent (icon, title, one sentence, one action).
8. **The artifact/side panel has a real header**: title, version or tab switcher, copy, download, close. Our dock's header is a chip strip plus loose glyph buttons.

## Cross-cutting

| # | Area | Keplorayx today | Claude | Change | P |
| --- | --- | --- | --- | --- | --- |
| X1 | Type | Inter everywhere; page titles 20 px regular | Sans for UI, serif for the greeting and page titles; titles ~24 px | Add a display face (or Inter Display / semibold 24 px) for page titles and the Home greeting only | P1 |
| X2 | Colour | Cool greys, blue accent; with a wallpaper the sidebar and dock now take its hue | Warm neutrals, one clay accent, never more than one accent per screen | Keep blue, but drop the blue on toggles to the accent at lower saturation, and use the accent only for the primary action and focus | P2 |
| X3 | Density | Rows 34–36 px, cards with 4 px insets, many hairlines | Rows ~36 px, fewer borders, more whitespace between groups | Remove card borders inside cards (Settings, Plugins); separate groups with 24 px of space and a caption | P1 |
| X4 | Relative times | `2962w` for anything older than a week (weeks forever) | "3 days ago", then a date | `ui::relative_time`: weeks up to 4, then `12 Oct`, then `Oct 2025` | P0 |
| X5 | Window chrome | Caption buttons sit right of the dock's own `⋯ ⤢ ✕` cluster, two sets of close buttons in one strip | One title bar, panel controls inside the panel header | Move the dock's `⋯ ⤢ ✕` into a dock header row under the bar (see T3) | P1 |
| X6 | Motion | Slides, fades, glyph morphs | Almost none; instant state changes, a short fade for panels | Keep, but cap page transitions at 120 ms and skip them when the target is already painted | P2 |
| X7 | Keyboard | Rich (Help lists 20+ shortcuts) | Fewer, discoverable through the `⋯` menus | Show each action's shortcut on the right of its menu row (we do in some menus) | P2 |

## Screen by screen

### Home (new thread)

| Keplorayx | Claude | Change | P |
| --- | --- | --- | --- |
| The folder card ("No folder / proj / Open another folder… Ctrl O", with a check) and a recent-thread card sit under the composer at all times. With the check mark and shortcut, it reads as a dropdown left open, and the pixel bird perched on it adds to the noise | Under the composer: suggestion chips, nothing else | Fold the folder card into the composer's folder chip (it already opens the same list). Keep "Continue: <thread>" as one quiet line, or drop it: Recent in the sidebar already shows it | P0 |
| Placeholder "Do anything…" is the only headline | Greeting in a large serif | Add a greeting line above the composer ("Good evening, Ravth"), wallpaper-aware contrast | P1 |
| Folder and device chips under the bar | Suggestion chips | Move folder/device into the composer row (they're settings for this thread); put 4–5 starter chips under it (Fix a bug, Review my changes, Write tests, Explain this repo, Just chat) | P1 |
| "2 agent updates · RAVTHEMAN" pill at the bottom | Nothing | Keep, but only when there are updates; it's good ambient info | P2 |

### Thread (chat)

| Keplorayx | Claude | Change | P |
| --- | --- | --- | --- |
| Composer: now the same card as Home, always open (no tucked "Send follow-up" pill) | Same composer everywhere | Done in this pass | — |
| Top bar: Chat / Terminal, info, then Diffs, Browser, Files and `⋯` (Terminal, History, Plan, Subagents, Chats, Side chat), then the panel toggle | Title left, a few actions right | Done in this pass. Next: highlight the bar button for whatever the dock shows (done) and give the `⋯` rows their shortcuts | P2 |
| User turns are right-aligned bubbles; replies are plain text with a copy glyph pinned top-right of the reply | Same bubbles; actions under the reply | Move the reply's copy glyph into the action row under the reply, with retry and fork | P1 |
| No "the agent can make mistakes" line | One faint line under the composer | Not needed for a dev tool. Skip | — |
| Thinking and tool calls collapse | Same | Keep | — |

### Dock (right panel)

| Keplorayx | Claude | Change | P |
| --- | --- | --- | --- |
| Empty dock: Run / Review / Agents launcher list | Artifacts panel only opens with content | Keep the launcher (it's how people find tools), but open it only from the bar, never on its own | P2 |
| Header: surface chips on the left, `+ ⊞ ⋯ ⤢ ✕` on the right, then a second row of glyph buttons (wrap, split, collapse, refresh) for Diffs | One header: title, switcher, actions, close | One header row: the surface title (or chips when several), then that surface's actions, then `⤢ ✕`. Fold the second glyph row into it | P1 |
| "Not a git repository" empty state with Initialize git / Open folder | n/a | Good. Keep | — |

### Search, Projects, Customize, More

| Keplorayx | Claude | Change | P |
| --- | --- | --- | --- |
| `DOVE_ROUTE=threads` and deep links land on Home: they set the route directly and skip the redirect to Search that the sidebar uses | `Ctrl K` / Search opens a centred palette over the page | Route both through `navigate`, so `threads` opens Search | P1 |
| Kanban, Automations, Pull requests, Usage, Archived, Team all highlight **More** in the sidebar | Each has its own entry or lives in Settings | Fine, but show the current page's name in the bar title (now blank on these pages) | P1 |
| Customize lands on Settings › Plugins (Built in, Apps, MCPs, Skills) | Settings › Connectors / Capabilities | Rename the page title to **Customize** when opened from the sidebar, so the label you clicked is the page you see | P1 |

### Settings

| Keplorayx | Claude | Change | P |
| --- | --- | --- | --- |
| Settings reopens on the last section used, remembered across restarts (`settings_section`) | Opens on General | Remember the section only within a session; a fresh launch opens General | P2 |
| The main sidebar is replaced by a settings nav with Back and Search settings | Same idea (left nav), no Back row | Keep. Make "Back" return to the page you came from (it does) and show `Esc` on it | P2 |
| Appearance: theme cards, then rows in a bordered card, then Material and background | Plain rows | Drop the outer card border; keep the theme cards | P1 |
| Agents: long list with path lines in monospace under each name | n/a | Hide the path behind hover or a details row; show only name, version and status by default | P1 |
| Plugins: four tabs with counts, search on the right, Browse directory + Add | Connectors: one list with "Add connector" | Good. Keep; align the tab underline with the title's left edge | P2 |
| Skills: category chips then toggles with long descriptions | n/a | Clamp descriptions to one line with a tooltip | P2 |

### Kanban, Automations, Pull requests, Usage, Archived, Team, Help, Flock

| Screen | Change | P |
| --- | --- | --- |
| Kanban | Column empty-state text is centred under each header in three lines; use one short line, left-aligned with the column title | P2 |
| Automations | The "Automations run while Keplorayx is open…" note is good; link "Keep running in the background" straight to that setting | P1 |
| Pull requests | Empty state shows a raw `git remote add origin https:/ /github.com/<you>/<repo>.git` that wraps mid-URL; put the command in a copyable code chip | P1 |
| Usage | "Export CSV" sits beside the range tabs as a second primary; make it a quiet text button | P2 |
| Archived | Good. Add a "Delete all archived" in the `⋯` of the header, with a confirm | P2 |
| Team | Strong page. Starter cards' descriptions run four lines; clamp to two | P2 |
| Help | Two columns of dense text; add a search box and group shortcuts (Threads, Panes, Panel) | P2 |
| Flock | The empty dock launcher shows beside the terminal; open Flock with the dock closed | P2 |

## Found while capturing (bugs, not taste)

- **`threads` deep link does not open Search** (P1, above).
- **Relative time in weeks forever** (`2962w`) (P0, above).
- The Home folder card looks like an open dropdown (P0, above).
- The first launch of a page in a fresh window sometimes takes over 10 s to show its window (seen once on Usage); worth a startup trace on a cold `DOVE_HOME`.

## Already done in this pass (2026-10-29)

- Left sidebar is Zeron's glass over the desktop blur; with a wallpaper, the sidebar and dock take the picture's dominant hue.
- A chat never shows the desktop while its wallpaper frost decodes.
- The thread composer is the Home card, always open.
- The thread bar shows the main dock tabs, a `⋯` for the rest, and the info button.
