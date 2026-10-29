# Dove UI/UX plan v5: the sidebar, Artifacts, and a UI people recognise

Written 2026-10-29 from a fresh pass over every Dove screen (current
`feat/ui-v4` build, scratch home) against the six reference shots of the
Claude desktop app. Builds on `ui-revamp-v4.md`; this plan replaces its
Phase 3 (sidebar) and adds Artifacts.

Screens captured (14): Home, Thread, Threads, Kanban, Flock, Automations,
Pull requests, Agents, Team, Usage, Archived, Help, Settings › General,
Settings › Appearance.

---

## 1. The sidebar, made the same as the reference

Same structure, spacing and behaviour as the reference sidebar, **minus**
its chat/code toggle and its menu (☰) button. The title bar keeps Dove's
own sidebar toggle and back/forward.

### 1.1 Top rows (fixed, never scroll)

| Row | Icon | Right side | Does |
|---|---|---|---|
| **New** | plus | `Ctrl+N`, always shown; hidden under 280 px | New thread. The only row with a resting fill. |
| **Search** | magnifier | — | Opens the search palette (`Ctrl+K`). Replaces the Threads page. |
| **Projects** | stacked folders | — | Opens the sidebar filter (section 1.3) on "Projects". |
| **Artifacts** | two overlapping cards | count when > 0 | Opens the new Artifacts page (section 2). |
| **Customize** | briefcase / palette | — | Opens Settings › Appearance. |
| **More** | `···` | — | Opens the More menu (section 1.4). |

- Rows are 40 px tall with a 20 px outline icon and a 16 px label, the
  reference sizes. Dove's rows are 41 px now and very close.
- Hover gives a full-width 8 px rounded fill. Selected gives a stronger fill.
  No pills or badges, except a count where one helps.
- **Removed from this list:** Threads (Search covers it), Kanban, Flock,
  Automations and Pull requests (all move into More).

### 1.2 The list under it (scrolls)

In order, each a collapsible section with a small muted header and a ▾ caret
that remembers its state:

1. **Pinned ▾**: pinned threads, each row with a *type icon* (code
   `</>`, chat bubble, automation sunrise, teammate avatar). Long titles
   **fade out** at the edge with a gradient mask instead of an ellipsis.
2. **One section per project ▾** (for example "Dove ▾"): rows have **no
   icon**, just the title. This is quieter than Pinned, as in the reference.
   - Idle or done rows in dim text; the active thread brighter.
   - Status as a coloured **word** on the right in place of the time
     (Waiting / Running / Failed). Dove already does this.
   - Trailing glyphs where they apply: branch/PR glyph (violet), remote
     cloud, `⑂ n` for several PRs.
   - Parents with children: a ▸ after the title and a muted
     `└ n nested sessions` line under it, expanding in place.
   - Hover reveals a `···` button (rename, pin, archive, move,
     delete). This replaces the always-visible archive icon.
3. **Chats ▾**: threads started with no folder (section 2.1), so plain
   chats never mix into a project.

- **Removed from the list:** the "Team 1" block and the yellow "Needs you"
  card. Waiting threads already say "Waiting" in amber in their own row,
  the card duplicated it, and the reference has neither. Teammates live in
  More › Team, and a teammate's threads show its avatar as their type icon.
- **Scrollbar:** an overlay scrollbar, 4 px, shown only while scrolling or
  hovering.

### 1.3 Filtering, all in the sidebar

One control, no extra page. A small **filter icon** on the right of the first
section header opens a popover. The **Projects** row opens the same popover.

- **Show:** All · Projects only · Chats only · one specific project
  (a list of projects with a check).
- **Status:** Any · Needs you · Running · Failed · Done.
- **Agent:** Any · Claude Code · Codex · … (only installed ones).
- **Group:** By project (default) · One list by recent.
- **Include archived:** a switch.

When a filter is on, the section header shows a quiet chip such as
"Dove · Needs you ✕" so it's obvious, and one click clears it. Choices
persist (`sidebar_filter.*`). The search box at the top of Search covers
text search, so the sidebar has no text field.

### 1.4 More menu

A popover anchored to the More row (12 px radius, raised fill, soft
shadow, 32 px rows, muted group labels):

- **Work:** Kanban · Flock (terminals) · Automations · Pull requests
- **People:** Team · Agents
- **History:** Archived · Usage
- **App:** Settings · Help & shortcuts

Keyboard: ↑ ↓ to move, ↵ to open, Esc to close. Every item keeps its
existing shortcut, shown on the right.

### 1.5 Footer

As in the reference: a round avatar initial, **name · plan** (for example "RAVTHEMAN
· Local"), and a chevron that opens the account menu (Settings, Theme,
Sign in / out, Usage today). A **feedback** glyph sits on the far right.
Today's "Today 0 tok" line and the two footer buttons move into that menu.
No divider above the footer, just space.

### 1.6 Removing the Threads page

- Delete `Route::Threads` and `pages/threads.rs`. The `threads` route id
  and `Ctrl+Shift+T` open **Search scoped to Threads** instead.
- Search's scope tabs become: **All · Threads · Artifacts · Projects ·
  Files · Actions**.
- Bulk actions (select several, archive, delete) move to Search: Shift-click
  rows, then a footer bar with Archive / Delete.

---

## 2. Artifacts (new)

What a thread *makes* when it isn't working in a project folder: a Markdown
doc, an article, an HTML page, a script, an image, any file. These items stay
on one page to look at again, like the reference's Artifacts.

### 2.1 No-folder threads

- Home's workspace card gets a first row: **"No folder: just chat"**
  (a speech-bubble glyph). The composer chip shows "No folder". Settings ›
  General gets "Default for new threads: Last folder / No folder".
- Such a thread runs in its own scratch directory,
  `~/.dove/artifacts/<thread-id>/`, so agents can still write files.
- The thread lists under **Chats** in the sidebar.

### 2.2 What becomes an artifact

- Every file a no-folder thread creates or edits (tool calls `Write`, `Edit`,
  `ToolDiff` with `old_text: None`), and anything the agent saves in its
  scratch directory.
- Optional, in project threads: a "Save as artifact" action on any
  Markdown/HTML/code block in the transcript, and on any file in Changes.
- Stored in a new `artifacts` table: id, thread, title (first heading or
  file name), kind (doc / page / code / image / data / file), path, size,
  created, updated, pinned. Files stay on disk; the table indexes them.

### 2.3 The page

- Header "Artifacts" with type tabs: All · Docs · Pages · Code · Images ·
  Files. A sort control (Recent / Name / Type) and a search field.
- A grid of cards, about 240 px wide, with:
  - **Preview:** rendered Markdown for docs, a live thumbnail for HTML,
    syntax-coloured head for code, the image itself for images.
  - **Text and actions:** title, kind · thread · age, and a `···` menu (Open,
    Open thread, Reveal in folder, Copy path, Save a copy…, Rename, Pin, Delete).
- Clicking a card opens it **full size in the side panel**: Markdown
  rendered (with Edit), HTML in the browser pane, code in the editor,
  images in a viewer. Each has a version list when the agent rewrote it.
- **Empty state:** a centred glyph and two muted lines, "Things your chats
  make show up here" / "Start a chat with No folder and ask for a doc, a
  page or a script."
- The **Search** palette's Artifacts tab searches titles and contents.

---

## 3. Customize

The **Customize** row opens Settings › Appearance directly, with the sidebar
showing the settings sections as today. Back returns to where you were.

---

## 4. What every screen needs (from the screenshots)

| Screen | Now | Change |
|---|---|---|
| **Home** | Close to the reference: workspace card, attention card, dove mascot, chips. | "No folder" row (2.1). The attention card's first row has no status word; show "Idle"/time in muted text so all rows align. |
| **Thread** | A GIF background plays raw **behind the text and over the sidebar** (the white squares show through the top of the sidebar). Hard to read. | Keep the art inside its own column (never under the sidebar). **Decision needed**: behind *working* screens (chat, Flock), a moving background should be frosted and dimmed like stills (my recommendation), with raw playback on Home only. |
| **Flock** | The raw GIF sits under the terminal and is cut off sharply at the side panel edge. | Same as Thread. Terminals always get the frost. |
| **Threads** | Columns don't line up: Status and Updated drift right on longer rows. | Page removed (1.6). |
| **Kanban** | Clean. | Cards use 12 px radius and a hairline like Home's cards; the column "+" shows only on hover. |
| **Automations** | Clean empty state. | Move the "runs while Dove is open" note into the empty card as its third line. |
| **Pull requests** | Clean. | The project picker matches the composer chip style. |
| **Agents** | Opens inside Settings (sidebar becomes the settings list). | Fine as a settings page; reach it from More › Agents. |
| **Team** | Good structure. | Presets as quiet chips without emoji. Stat numbers 20 px, not display size. |
| **Usage** | Clean. | Bar colour from the accent token; hover shows the day's value. |
| **Archived** | Clean. | Merges into Search with an "Include archived" filter; the page stays for bulk clean-up. |
| **Help** | "Split pane down" shows `Ctrl Alt` with **no key**. | Fix the keycap (missing arrow glyph). Rows 36 px, keycaps right-aligned in one column. |
| **Settings** | Cards look right. | Left list uses the same row style as the new sidebar. |

---

## 5. What makes the UI "recognised for being perfect"

The bar is consistency and calm, not more features. Every screen should
look as if one person drew it on one day.

1. **One system, enforced.** Every surface draws from the tokens in
   `design.rs` / `theme.rs` (four greys, five radii, three type sizes,
   one icon set at 20 / 16 px). A CI check fails on raw colours, radii
   or font sizes outside those files.
2. **Status as words, actions on hover.** No badges and no permanent icon
   clutter. A row shows what it is and what needs you; everything else
   appears on hover or in `···`.
3. **Keyboard first.** Every action is in the palette, and every list moves
   with ↑ ↓ ↵. Focus rings show only for keyboard focus.
4. **Instant.** Every click paints within one frame; nothing blocks the UI
   thread. Budgets: cold start under 1 s, opening a thread under 100 ms, idle CPU under 1 %.
   Check them with `perf_trace` on each release.
5. **Motion with meaning.** Panes slide 180 ms, popovers fade and rise
   120 ms, hover 80 ms. All respect Reduce motion (except backgrounds, by
   your choice).
6. **Readable everywhere.** Text holds 4.5:1 contrast on every plane,
   including over any wallpaper. That's why working screens get the frost.
7. **Empty states that teach.** Each empty page says what fills it and
   offers the one action to start.
8. **Screenshot gate.** The capture script that took these 14 shots runs
   on every UI change. A reviewer compares before and after side by side
   before merging.

---

## 6. Order of work

| # | Work | Files | Size |
|---|---|---|---|
| 1 | Sidebar top rows, More menu, footer; remove Team block and Needs-you card | `sidebar.rs`, `kit/popover.rs` | M |
| 2 | Sidebar sections: Pinned with type icons, project groups, Chats, nested lines, hover `···`, fade-out titles, overlay scrollbar | `sidebar.rs`, `kit` (`fade_text`) | M |
| 3 | Sidebar filter popover + persisted filters + active-filter chip | `sidebar.rs`, `store` settings | S |
| 4 | Remove Threads page; Search scopes and bulk actions | `shell.rs`, `palette.rs`, `pages/threads.rs` | S |
| 5 | Customize → Appearance | `sidebar.rs`, `shell.rs` | XS |
| 6 | Fix moving backgrounds on working screens (after your decision) and keep art out of the sidebar | `shell.rs`, `wallpaper_media.rs` | S |
| 7 | No-folder threads (scratch dir, composer chip, Home row, setting) | `dove-core`, `home.rs`, `composer` | M |
| 8 | Artifacts: table, capture from tool events, page, side-panel viewer, Search tab | `dove-store`, `dove-core`, new `pages/artifacts.rs`, `side_panel.rs`, `palette.rs` | L |
| 9 | Per-screen fixes from section 4 | various | S each |
| 10 | Token check in CI, perf budgets, screenshot gate | `scripts/`, CI | M |

Items 1–5 make the sidebar match the reference and can land together.
Item 8 is the largest and can run in parallel once item 7 is in.

## 7. Decisions (answered 2026-10-29)

1. **Moving backgrounds:** frosted and dimmed like stills behind chats and
   terminals; sharp (lightly dimmed) on Home only. Art never under the
   sidebar.
2. **Footer label:** "name · Local" now, the plan name once accounts exist.
3. **Team:** in the More menu, and a teammate's threads show its avatar as
   their type icon.
4. **Scope:** build the whole plan (sections 1–6).
