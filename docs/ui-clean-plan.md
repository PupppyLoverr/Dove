# Clean like Claude: the Keplorayx UI clean-up plan

Written 2026-10-29. Refines [ui-ux-claude-comparison.md](ui-ux-claude-comparison.md) using two reference screenshots and a read-only look at Claude's desktop app (Code tab) on this PC.

**Keep, untouched:** the K22 icon set; Glass, Acrylic and Normal materials; wallpaper effects (halftone etc.), animated GIF/video backgrounds, custom backgrounds, the chat frost; the wallpaper-hue tint on the sidebar and dock; the notch. **Change:** layout, density, typography, how many surfaces and borders a screen shows, and where controls live.

## 1. The two reference images

### Claude (desktop app, Code tab)

- **Top bar, one row, edge to edge.** Left: a menu glyph (≡), the sidebar toggle, back and forward, a two-segment **Chat | Code** switch, then the session title ("RE") with a ⌄ that opens the session's menu. Right: five panel toggles (summary card, terminal, diffs, browser, files) and ⋯. The one whose panel is open (Files) sits on a filled square. Then the window controls. No dividers, no labels, no second set of close buttons.
- **Sidebar.** New (a small dot on the right marks activity), Search, Projects with a small **Beta** pill, Artifacts, Customize, More. Then collapsible group headers in small grey text: **Pinned ▸**, then one group per project (**Dove ▾**). Sessions are single lines, title only: no agent name, no time. Nested sessions hang off their parent on a thin indent line, collapsed to "25 nested sessions". Small coloured status glyphs (branch, cloud) sit at the far right. Footer: avatar, "Kamal · Max ⌄", and one icon.
- **Transcript.** One column about 800 px wide, comfortable 15–16 px text, real markdown (bold lead-ins, numbered and nested lists, inline code chips). Under each reply, a quiet row of four glyphs (copy, fork, pin, read aloud) and "51 minutes ago". The only card on the page is the **next-step card**: a hairline-bordered rounded box with a short recap, one question and one or two buttons (the first filled), and a ✕.
- **Composer.** A single-line pill with + on the left, "Type / for commands", and the mic on the right. It grows as you type. Under it, outside the pill: the permission mode on the left; model, effort and the plan badge ("Opus 5.5 Medium · Ultracode") and a small context ring on the right.
- **Right panel (Files).** One header row: the title "Files" on the left; search, view, ⋯, expand and ✕ on the right. Below it, a calm centred empty state ("Open files appear here" plus one line), and the file tree on the right with a filter field. The panel is set off from the chat by tone and one hairline, not a card.
- **Overall.** Two near-black tones (chat and panel a shade darker than the sidebar), hairlines instead of borders, one accent at a time, generous spacing, every row 36–44 px. Nothing glows, nothing is outlined twice.

### Keplorayx (thread, today)

- **Top bar.** Left: the sidebar toggle, back, forward and +. Then the agent's glyph and the title ("Browser Control Inquiry"). Right of the chat: Chat/Terminal (Chat filled), terminal, info, Diffs, Browser, Files and ⋯. Then the dock's own ⋯, ⤢ and ✕, then the window's – ☐ ✕. **Two ⋯ menus and two ✕ sit side by side.** The title has no menu.
- **Sidebar.** New on a filled pill, Search, Projects, Artifacts, Customize, More. One **Recent ⌄** group. The selected row shows the title and "3h". Footer: avatar, "RAVTHEMAN · Local ⌄", one icon. The surface is navy-tinted glass (the wallpaper's hue).
- **Transcript.** Tool activity in **bordered pills** ("Read 1 file"), a warning row ("Read C:\…png · Outside… Allowed"), "Thought process" collapsed, then the reply. A long URL in an inline code chip wraps over two lines. Copy, retry and fork sit under the reply, with no time.
- **Composer.** The tall Home card (about 150 px): "Send follow-up" over a row with paperclip, "Bypass permissions", the model ("Haiku 5.5 Low"), call, mic and a send disc. Under the card: the folder ("dove") on the left and the context ring ("10%") on the right.
- **Dock.** No header or title. The launcher list (Run: Browser, Terminal; Review: Diffs, Files, History, Plan; Agents: Subagents, Chats, Side chat) and "Ctrl+\ hides the panel". It is translucent, so the wallpaper shows through it.
- **Overall.** Three materials on one screen (solid chat, navy-tinted sidebar, see-through dock), boxed tool rows, two clusters of the same controls, and a composer that outweighs the reply above it.

## 2. What makes Claude's version read as clean

1. **One control per job.** A single panel cluster, a single ⋯, a single ✕ per surface.
2. **Two surface tones, not three materials.** Chrome (sidebar, bar) and content (chat, panel) differ by one step; panels are content, not chrome.
3. **No boxes for activity.** Tool calls and thoughts are text rows with a chevron, not bordered pills.
4. **A light composer.** One line until you type; settings live under it, outside the input.
5. **Headers carry titles.** Every panel says what it is, and its actions sit in that one row.
6. **Grouping instead of decoration.** The sidebar groups by Pinned and project; rows are title-only.
7. **One rhythm.** 16 px glyphs on 32 px targets in the bar, 36–40 px rows, 15 px body text, 13 px meta.

## 3. Rules that keep our materials

These rules make the clean-up work on every theme:

- **Materials apply to chrome; content gets a plane.** Glass, Acrylic and Normal style the sidebar and top bar (and the wallpaper's hue tints them). The chat **and the dock** sit on the same content plane: solid on Normal, and on Glass/Acrylic the chat's frost (wallpaper on) or `panel_bg` (wallpaper off). The dock stops being a third material.
- **The wallpaper shows in one place per page.** Home: the hero art behind the composer. Thread with "Show behind chats": one continuous frost under chat and dock. Off: the content plane. The sidebar only ever takes the hue.
- **One hairline weight** (`t.wash(0.06)`) for every seam; no border on a surface that already has a different tone.
- **Text over art** keeps the existing contrast-solved scrims; no new per-component scrims.

## 4. Work packages

Each one is a single commit, testable on its own, in this order.

| # | Package | What changes | Files | Done when |
| --- | --- | --- | --- | --- |
| C1 | **One bar** | Remove the dock's `⋯ ⤢ ✕` from the top bar (they move to C2's dock header). Make the Chat/Terminal switch a two-segment pill like Claude's. The bar's panel buttons are the five Claude has: summary (info), terminal, diffs, browser, files; History, Plan, Subagents, Chats and Side chat go under ⋯. The open one sits on a filled square. Title gets a ⌄ menu: rename, pin, fork, archive, copy path, open folder (the thread context menu, reused) | `shell.rs` `render_topbar`, `render_dock_tabs`; `side_panel.rs` `render_strip`; `thread.rs` `view_switch` | Screenshot: one ⋯ and one ✕ per surface in the bar |
| C2 | **Dock header** | One row inside the dock: surface title (or chips when several are open), that surface's own actions (search, view, wrap, refresh), then `⤢ ✕`. The second glyph row (Diffs' wrap/split/collapse/refresh) folds into it. The empty dock shows the launcher under a "Panels" title | `side_panel.rs` | Every dock surface has a titled header; no row of loose glyphs |
| C3 | **Content plane** | The dock uses the chat's plane: solid on Normal, `panel_bg`/frost on Glass and Acrylic, and continuous frost with the chat when the wallpaper is behind chats. The sidebar keeps its glass and wallpaper hue | `shell.rs` (`chrome`, `dock_layer`), `theme.rs` | Thread screenshot shows two tones (chrome, content) on all three materials, with and without a wallpaper |
| C4 | **Quiet activity** | Tool groups ("Read 1 file") and "Thought process" become borderless rows: chevron, verb in medium weight, target in muted text, status at the right. A permission note ("Outside… Allowed") becomes muted meta text on the tool row, not its own warning line. Inline code chips use `white-space: nowrap` with ellipsis; long URLs become links | `transcript.rs`, `kit.rs` | A long thread reads as text with a few chevrons, no boxes |
| C5 | **Reply row** | Under each reply: copy, retry, fork, and the time ("51 minutes ago") in meta grey, shown on the latest reply and on hover for older ones (we already do the hover part) | `transcript.rs` | Matches Claude's row |
| C6 | **One composer** | Decision needed (see §5). Recommended: Claude's model everywhere. A one-line pill that grows as you type, never tucked or idle: + (attach, mode, skills, @), the prompt, mic and send inside. Under it, outside: mode on the left; model + effort and the context ring on the right; folder and device move into the + menu and the thread's title menu | `composer/mod.rs` (`render_body`, `render_dock`), `home.rs` | Home and thread show the same composer; it's ≤ 52 px tall when empty |
| C7 | **Sidebar groups** | Groups: Pinned, then one per project (collapsible, project name in small grey), then "Chats" for no-folder threads. Rows title-only; agent glyph and time move to the row's tooltip and hover. Nested threads (subagents, side chats) hang off the parent on an indent line, collapsed to "N nested". A dot on New when something needs you. A "Beta" pill on Projects | `sidebar.rs` | Screenshot matches Claude's structure |
| C8 | **Home** | Remove the always-open folder card and the pixel bird perched on it. Above the composer, a greeting ("Good evening, Ravth"), 28 px semibold, contrast-solved over the art. Under it, 4–5 starter chips. "N agent updates" pill only when there are updates | `home.rs` | Home is greeting, composer, chips |
| C9 | **Type and rhythm** | Body 15 px, meta 13 px, page titles 24 px semibold, bar glyphs 16 px on 32 px targets, sidebar rows 36 px, transcript column 800 px max. Audit with the Theme tokens; no per-screen sizes | `design.rs`, `ty`, `ui.rs` | One set of numbers; pages match |
| C10 | **Pages** | Apply the comparison doc's page rows: Settings without cards-in-cards, Agents without path lines by default, PR empty state with a copyable command, Customize titled "Customize", relative times turning into dates after four weeks, `threads` deep link opening Search | per page | The P0/P1 rows in the comparison doc are closed |

**Order:** C1 → C2 → C3 make the thread screen match the reference, so they go first, with one screenshot review after C3. Then C4–C6 (the transcript and composer), then C7–C8 (sidebar and Home), then C9–C10 as a sweep.

**Each package:** build into `C:\dvw\target-ui-v4`, screenshot Home and a thread on Glass, Acrylic and Normal, with and without a wallpaper (six shots), and compare against the Claude reference before committing.

## 5. Decisions for you

1. **Composer:** you asked earlier for the thread composer to match Home's tall card and never go idle. Claude instead uses one slim pill everywhere that grows as you type, and is never idle either. Recommended: the slim pill on Home and in threads (C6). Alternative: keep the tall card everywhere and only move the footer chips into it.
2. **Bar buttons:** Claude shows five panel toggles. Ours would be info, terminal, diffs, browser and files, with the rest under ⋯. OK to drop History and Plan from the bar?
3. **Sidebar times:** Claude shows no times on rows. Drop ours ("3h") into the tooltip, or keep them?
