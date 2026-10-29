# Keplorayx UI/UX revamp — plan v4 ("quiet desk")

Status: proposal, 2026-10-28. Builds on `ui-revamp-plan.md` (v2) and `revamp-v3/`.
Where this plan and v2 disagree about the window chrome (v2's thread-tab top bar),
this plan wins; v2's chat-speed and chat ⇄ terminal work stand.

Source: six reference screenshots of a desktop agent app (new-session screen,
sidebar, files pane with a placement menu, context panel, search palette,
empty state). We take the **design principles and interaction patterns**,
not the look one-to-one: no copied mascot, wording, icon set or exact
layout. Keplorayx keeps its own mark, Solar icons, accent, copy and the custom
wallpaper, which the references don't have.

Design happens in **Paper** first (paper.design, local MCP on :29979), one frame
per screen below, before any Rust changes.

---

## 1. What the references show, image by image

### Image 1 — the whole window, new session, empty

**Title bar (about 46 px, same fill as the sidebar, no divider below it)**
- Left group: menu (three lines), sidebar toggle (panel glyph), Back `<`, Forward `>`.
  All 20 px outline icons with no button chrome until hovered.
- A **two-segment mode switch** (chat bubble with a plus / `</>` code). The active
  segment gets a raised fill, about 8 px radius. The switch sits flush right
  of the sidebar column.
- The **page title** ("New session") in muted grey, about 15 px, left-aligned
  to the content column, not centred.
- Right group: terminal `>_`, globe (browser), folder (files). These are
  **pane toggles**: the active one gets a filled square (see Image 3).
- Native-style caption buttons (min, max, close): thin glyphs, no fill.

**Sidebar (about 325 px, one shade lighter than the canvas)**
- Primary nav, each row about 40 px tall, 20 px icon plus about 16 px label:
  - **+ New** with a `Ctrl+N` hint right-aligned and always visible. Its row has
    a resting fill: the only filled nav row, so it reads as the primary action.
  - Search, **Projects** with an outlined **"Beta" pill**, Artifacts, Customize, **More (…)**.
- **Section header "Pinned ▾"**: about 13 px, semibold, muted, with a collapse caret.
  - Pinned rows carry a **type icon**: camera (social or scheduled), a green
    sunrise (scheduled routine), `</>` (code session).
  - Long titles **fade out** at the right edge (a gradient mask, no ellipsis).
- **Project group "Keplorayx ▾"**: rows have **no icon**, just text. That hierarchy is
  quieter than Pinned.
  - An archived or idle row ("Worktrees cleanup and sync") shows in **dimmed text**.
  - Parents with children have a **▸ disclosure** after the title. Under them sits
    a **"└ 2 nested sessions"** line, indented with a hairline tree connector
    and smaller muted text: a collapsed child count instead of listing every child.
  - Trailing **status glyphs**: purple branch or PR glyph, cloud glyph (remote
    session), `⑂ 2` (count of PRs or branches) in purple.
- Thin native-looking scrollbar with arrow caps. It's visible here, which feels
  a little dated (see the plan).
- **Footer**: round avatar initial, name · plan tier ("Max", muted), chevron for
  the account menu. A bug glyph on the right for feedback. No divider, just space.

**Main canvas (darkest plane)**
- **Empty state, optically centred a little below the middle:**
  - A small **pixel-art mascot** perched on the top-right edge of the first card:
    character and warmth in one accent colour (orange).
  - **Environment card**: a list of workspaces (laptop glyph plus name: Downloads,
    dove ✓, zeron), a divider, then **"Choose environment ⌄"**. About 400 px wide,
    12 px radius, 1 px low-contrast border, fill a hair lighter than the canvas.
  - **Attention card** under it: three recent sessions, one line each:
    - **"Waiting"** as a coloured *word* (amber), not a badge, followed by the
      title and a muted trailing reason ("Asked a quest…").
    - "Turn on Worktree" plus muted "Session running here".
    - A plain row, truncated with an ellipsis.
- **Composer**, pinned near the bottom, about 960 px max width, about 54 px tall,
  about 14 px radius, a lighter fill than the canvas, a hairline border:
  - `+` (attach) on the left, placeholder text, mic on the right. **No send button
    until there is text.**
- **Status chips under the composer**, with no fills, just icon plus text in muted white:
  - left: laptop "dove" (environment), branch "Worktree" (isolation mode),
    "Bypass permissions" (permission mode);
  - right: **"Opus 5.5" plus a muted "Medium"** (model plus effort), and a small
    **circular context ring**.

### Image 2 — the sidebar alone (hover states)
- **Row hover** gives a full-width rounded fill (about 8 px) and reveals a trailing
  **"…" more button**. Actions stay hidden until hover, so rows stay clean.
- "+ New" without its shortcut hint: the hint drops when the column is narrow.
- The same group, nesting and status-glyph language as Image 1.

### Image 3 — the files pane open, with the placement menu
- The right pane takes about 35 % of the width. The **chat column re-centres** in
  what's left, and the composer narrows with it.
- Pane header: title "Files" on the left; right: search, **list or tree toggle**
  (active one filled), "…", **expand ↗** (take over), close ×.
- **Placement popover** (from "…"):
  - "Move": four glyphs to dock **left, right, top, bottom**; the current one is highlighted.
  - "Fill and arrange": **fill the window** or **split side by side**.
  - "Open in new window" row with a pop-out glyph.
  - About 12 px radius, raised fill, soft shadow, small grey section labels.
- The pane body is split: an **empty preview area** ("Open files appear here /
  Pick a file in the tree", a centred folder glyph, two lines of muted text)
  and a **file tree** on the right: chevrons, folder names, per-type file icons
  (Markdown and TOML get distinct glyphs), dotfiles listed.
- The title-bar folder toggle shows as **active** (filled square).

### Image 4 — the context panel (session details)
Stacked **collapsible cards** (about 14 px radius, hairline border, a hair lighter fill):
- **Environment ▾**: machine ("dove") with a gear on hover; folder "Keplorayx ·
  C:\Users\…\Downloads" (name bright, path muted); branch "feat/notch-v2";
  **Remote Control** with a toggle switch.
- **Context ▸ 12 %** with a tiny inline progress bar (blue): collapsed to one line.
- **Connectors ▸ "3 to reconnect"**: collapsed and showing only what needs action.
- **Changes ▾** with a **drag handle ⠿** (cards can be reordered): `+15,246`
  green, `−256` red, and a secondary **Create PR** button.
- The principle: every card is one line when collapsed and states the
  one number or problem that matters.

### Image 5 — the search palette
- A centred modal, about 800 × 660, about 16 px radius, a **lighter** fill than the
  window, and a dim scrim behind it.
- Header: input "Search" with a × close.
- **Scope tabs**: All (active pill), Artifacts, Sessions, Code, Projects, Scheduled.
- **"Recents"** section: rows with type icons (code, chat, camera); the selected row
  is filled and shows **"…" (actions) and "↵"** on its right.
- **"Actions"** section: "New session".
- **Footer key legend** with keycaps: Close `Esc`, Change type `← →`, Filters `/`,
  Actions `Alt ↵`; right: Compose `Tab` (send the query to a new session).

### Image 6 — the empty state, zoomed and focused
- The mascot is in a **different pose**: it's animated or idles between poses.
- The list is **reordered with the current environment first** (dove ✓ on top):
  recency or selection ordering.
- The composer has **focus** (caret shown, border a touch brighter): it
  autofocuses on a new session.

### Design-system read-out (shared by all six)
| Aspect | What they do |
|---|---|
| Colour | Neutral greys only (about 4 steps: canvas ≈ #1a1a1a, sidebar or card ≈ #1f1f1f, raised ≈ #2a2a2a, hover ≈ #2e2e2e). Colour is reserved for *meaning*: amber waiting, purple git, green and red diff, blue progress, orange mascot. |
| Borders | 1 px, very low contrast. Cards separate by fill more than by line. |
| Shadows | Only on floating things (popover, palette). Planes are flat. |
| Radius | About 8 rows and buttons · about 12 cards and popovers · about 14 composer and panel cards · about 16 modal. |
| Type | One sans; about 16 rows, about 15 title, about 13 section labels and meta; weight does the hierarchy, not size jumps. |
| Density | Generous 40 px rows, wide gutters, lots of empty canvas. |
| Icons | One outline set, about 1.5 stroke, 20 px in chrome, 16 px in rows. |
| Status | Words and small glyphs, not badges and pills (except "Beta"). |
| Disclosure | Hover reveals actions ("…", gear); collapsed counts ("2 nested sessions", "3 to reconnect"). |
| Motion | Implied: mascot idle, smooth pane reflow. |

---

## 2. Where Keplorayx is today (relevant code)

| Area | File(s) | Today |
|---|---|---|
| Tokens | `apps/dove/src/design.rs` | Zeron rhythm: 10 px radius everywhere, 26 composer, type 11–22 px. |
| Theme and material | `theme.rs` | Glass, Acrylic and Normal; `GLASS_ALPHA 0.80`, `composer_blur` 16 or 30. |
| Chat wallpaper | `shell.rs:51-56`, `wallpaper_layer` (~`shell.rs:3133`) | `THREAD_ART_BLUR = 30`, `THREAD_ART_TINT = 0.60`, live frost over the whole column. **This mid-strength blur is the smeary "not neat" look**: shapes are half-dissolved, so they read as dirt rather than colour. |
| Wallpaper pipeline | `wallpaper.rs` (1,946 lines) | Stills only (png, jpg, webp), effects Dither, Halftone and Scanlines, baked `soften()` at quarter size, an animated *effect* (not animated media). |
| Settings | `settings.rs:225-227`, `:1326-1560` | `wallpaper_path`, `wallpaper_effect`, `wallpaper_animate`. |
| Sidebar | `sidebar.rs` | Groups, attention sort, nested threads (`nest`), agents section. |
| Home | `home.rs` | Hero plus composer, agents and updates pills, onboarding. |
| Composer | `composer/mod.rs` | Target, git and context rows, queue, attachments, mention picker. |
| Right dock | `side_panel.rs`, `files_panel.rs`, `split.rs`, `changes.rs` | Dock at 520 px, `panel_fit` responsive hiding. |
| Palette | `palette.rs` | Command palette. |
| Popovers and kit | `kit/popover.rs`, `kit/controls.rs`, `kit/page.rs` | Shared controls. |

Zeron's reference recipe (`../zeron/crates/ui/src`): `frost.rs` `MENU_BLUR = 16`
for floating cards; `theme.rs` `GLASS_ALPHA 0.80` over **OS-level** acrylic or
vibrancy (a very wide blur *with a material*), plus `contrast_checked_tint_alpha`
so text contrast is guaranteed over any backdrop. Zeron's wallpaper reads
clean because the blur is wide enough to leave only colour, and the scrim is heavy and
contrast-checked.

---

## 3. The plan

### Phase 0 — Paper designs (gate for everything below)
One Paper page, "Keplorayx v4", with frames at 1440 × 900 and 1920 × 1080, dark and light:
1. New session, no wallpaper · 2. New session with wallpaper · 3. Thread with wallpaper (new frost)
· 4. Thread plus Files pane · 5. Placement popover · 6. Context panel · 7. Search palette
· 8. Settings › Appearance › Background · 9. Sidebar states (hover, nested, collapsed, narrow).
Tokens are defined in Paper first, then mirrored in `design.rs`. Sign-off before Phase 1.

### Phase 1 — Tokens and theme (`design.rs`, `theme.rs`)
- **Neutral ramp** with 5 named planes: `canvas`, `side`, `card`, `raised`, `hover`
  (dark about 6.5 %, 8 %, 9 %, 12 %, 14 % L; light mirrored). Every surface uses these
  names, with no ad-hoc greys. Audit with a Haiku sweep: `grep grey(` and `neutral(`.
- **Radius scale**: `ROW 8 · CARD 12 · PANEL 14 · MODAL 16 · COMPOSER 16` (down from 26:
  quieter, less pill-like). Keep `radius::fit` and `nested`.
- **Type**: bump `BODY` 13 → 14 and `MD` 14 → 15, sidebar rows 14.5, section labels 12.5
  semibold muted. Weight carries hierarchy.
- **Borders**: `hairline` (about 6 % white) and `hairline_strong` (about 10 %); cards separate
  by fill first.
- **Semantic colours** (used only for meaning): `waiting` amber, `git` violet,
  `added` green, `removed` red, `progress` Keplorayx accent, `danger`. The Keplorayx accent stays
  Keplorayx's own (not orange), and the mascot uses it.
- **Elevation**: `flat` (planes), `float` (popover: blur 16, shadow y4 b24 at 0.35),
  `modal` (palette: plus a 0.45 scrim).
- Row height `ROW_H = 36` (compact density 32, comfortable 40) via the existing size prefs.

### Phase 2 — Window chrome (`shell.rs`, `surface_chrome.rs`, `shell_menu.rs`)
- Title bar 44 px, the same plane as the sidebar, no bottom border.
- Left: menu · sidebar toggle · back or forward (history of routes, new: a `NavHistory`
  stack in `Shell`) · **mode switch** (Keplorayx's own modes: *Chat* / *Code*, where Code opens
  the session's terminal or TUI view from v2's chat ⇄ terminal toggle).
- Page title left-aligned to the content column (thread title, editable on double-click).
- Right: **pane toggles** Terminal · Browser · Files · Details (context panel), each a 28 px
  square that fills when its pane is open; tooltips with shortcuts.
- Windows caption buttons redrawn with thin Solar glyphs and a red hover on close only.
- Drop the v2 thread-tab strip (tabs move into the palette and sidebar).

### Phase 3 — Sidebar (`sidebar.rs`)
- Primary nav: **New** (filled resting row plus `Ctrl+N` hint, hidden under 280 px),
  Search (opens the palette), **Projects**, **Team** (never "Crew"), Automations,
  Plugins, More (…) → Kanban, Pull requests, Usage, Archived.
- Sections: **Pinned ▾** (type icons), then **one group per project ▾** (no icons on
  rows, quieter), then **Agents** (running teammates) if any. Sections collapse and remember.
- Rows: 36 px, 8 px radius hover fill, **title fades at the edge** (gradient mask;
  a new `kit::fade_text`), trailing status glyphs (git violet, remote cloud,
  `⑂ n`, an unread dot), and **"…" revealed on hover** (rename, pin, archive, move).
- Status as **coloured words** in the row's meta line ("Waiting", "Running", "Failed"),
  replacing badge pills.
- **Nested sessions**: the parent shows ▸ and the child line "└ n nested sessions"
  (connector hairline). Expand inline. Reuses `nest()`.
- Idle or archived rows dim to `text_faint`.
- Overlay scrollbar (appears on hover or scroll, 4 px thumb, no arrow caps).
- Footer: avatar plus name · plan, an account menu (settings, theme, sign-in), and a
  feedback glyph on the right.

### Phase 4 — Home or new session (`home.rs`, `intro/`)
- Layout: content column max 720 px; cards optically centred at about 46 % height;
  composer docked at the bottom (keep the `HOME_INPUT` dock or inline setting).
- **Workspace card**: recent workspaces (current first, ✓), divider, "Open folder…"
  and "Clone repository…" rows. A hover gear per row.
- **Attention card**: up to 3 sessions needing you, sorted by `attention_rank`,
  each a coloured status word plus title plus muted reason; click opens the session.
  Hidden when there are none.
- **Keplorayx's mark**: an original pixel dove (our own sprite sheet, 3–4 idle poses,
  blinks or preens every 6–10 s, still under reduced motion) perched on the
  workspace card's edge. Not a crab, not orange: Keplorayx accent.
- With a wallpaper: the hero keeps its effects and the fade into the canvas; the
  cards get the `float` frost so they sit cleanly on the art.
- The composer autofocuses on New (`Ctrl+N`).

### Phase 5 — Composer (`composer/mod.rs`, `composer/picker.rs`)
- Plate: max 880 px, min height 52, `COMPOSER` radius 16, `raised` fill on glass at
  `float` frost, hairline border → `hairline_strong` on focus (no glow ring).
- Inside: `+` attach menu (files, image, @mention, skill) · text · mic · **send
  appears only with content** (cross-fade, 120 ms) · stop while running.
- **Chip row under the plate** (no fills, muted): workspace · isolation (Local /
  Worktree / Cloud) · permission mode · *right:* model + muted effort · context
  ring (progress turns amber at 80 %, red at 95 %). Each chip opens its popover.
- These replace today's in-plate target, git and context rows (they move to chips).

### Phase 6 — Thread and transcript (`transcript.rs`, `tool_calls.rs`, `markdown.rs`, `thread.rs`)
- Column max 760 px, centred in what remains when panes open (Image 3 reflow).
- User messages: right-aligned `card` bubble, 12 px radius. Assistant: no bubble, plain text.
- Tool calls: one-line collapsed rows (glyph · verb · target · duration), expanding
  to detail; groups of more than 3 consecutive calls fold into "Ran 7 tools ▸".
- A status strip above the composer (24 px, reserved): "Working · 12 s" shimmer.
- A bottom fade band so text dissolves under the composer (Zeron edge-fade
  technique, which works over a blurred backdrop).
- Permission prompts: an inline card with clear primary and secondary buttons.

### Phase 7 — Right panes (`side_panel.rs`, `files_panel.rs`, `split.rs`, `changes.rs`)
- One **pane host** with a shared header: title · tool icons · "…" · expand · close.
- **Placement menu** ("…"): dock left, right, top or bottom · fill the window ·
  split side by side · open in new window (a new GPUI window hosting the same entity).
  Persist per pane kind. Extends `panel_fit` with top and bottom docks.
- **Files**: tree and preview split (the tree can sit left or right), an empty preview state
  ("Pick a file to preview"), list or tree toggle, search, per-type icons
  (`file_icons.rs`), dotfiles shown dimmed.
- **Details** (the context panel, new): reorderable collapsible cards with drag handles:
  *Environment* (machine, folder, branch, Remote toggle) · *Context* (percentage and bar,
  expands to a token breakdown) · *Integrations* ("n to reconnect") · *Changes* (+/−,
  Create PR, file list) · *Team* (teammates on this session). Each card is one
  line when collapsed and shows only the actionable number.
- Terminal and Browser panes adopt the same header.

### Phase 8 — Search palette (`palette.rs`)
- Modal 760 × 600, `MODAL` radius, `raised` fill plus `modal` scrim.
- Scope tabs: **All · Sessions · Projects · Files · Automations · Commands**, switched with ← →.
- Sections "Recent" and "Actions"; the selected row shows "…" (an Alt+↵ action menu) and ↵.
- **Tab = start a new session with the query** (Keplorayx's version of compose).
- A footer keycap legend (a new `kit::keycap`). `/` opens filters (project, status, agent).

### Phase 9 — Popovers, menus and controls (`kit/popover.rs`, `kit/controls.rs`)
- Every float uses `float` elevation, a 12 px radius, 6 px inner padding, 32 px rows,
  muted section labels, and keyboard navigation.
- New kit pieces: `segmented` (the mode switch and list toggle), `keycap`, `fade_text`,
  `status_word`, `collapsible_card` (with a drag handle), `overlay_scrollbar`,
  `icon_toggle` (pane toggles).

### Phase 10 — Settings and other pages (`settings.rs`, `pages/*`, `kit/page.rs`)
- Settings as a two-column page: left section list (the same row style as the sidebar), right
  cards grouped like the Details panel. Search stays.
- Pages (Agents, Automations, Kanban, Plugins, Usage, Archived, Threads) share
  `kit::page`: a 44 px header, a 760–960 px column, and empty states built from the
  "glyph + two muted lines" pattern.

### Phase 11 — Backgrounds: keep custom, add animated, fix the chat blur

**11a. Keep everything that exists**: still images, effects (Original, Dither,
Halftone, Scanlines), the animated effect, and the home hero fade.

**11b. Animated wallpapers (new)**
- Formats: **GIF, APNG, animated WebP** (decoded with the `image` crate, already a
  dependency with `gif` and `webp`), and **MP4 or WebM** via ffmpeg when present:
  transcoded once into a frame cache (≤ 30 fps, ≤ 1920 px long side) under
  `DOVE_HOME/wallpapers/cache/<hash>/`. Without ffmpeg the picker greys out
  video formats and offers to install it.
- Pipeline (`wallpaper.rs`): a new `Media::{Still, Frames{frames, delays}}`. Decode
  off the main thread; frames become `RenderImage` frames; playback is paced by
  `request_animation_frame` using frame delays.
- **Performance rules** (from Keplorayx's perf-first stance):
  - Pause when the window is unfocused, minimised or occluded, on battery saver, or under reduced motion.
  - Cap at 30 fps; the home hero plays at full resolution.
  - **The chat backdrop plays pre-blurred, low-res frames** (1/8 size: after a
    wide blur the detail is gone anyway), so an animated chat background costs
    little memory and no live blur pass. Budget: ≤ 64 MB of frames, ≤ 1 % CPU at
    idle; frames beyond the budget are dropped (decimated) rather than overrunning it.
  - Effects on animated media: Original only in v4 (per-frame dither is a later item).
- Settings › Appearance › **Background**:
  - Source: None · Image · Animated · (later) Folder rotation.
  - A recent-backgrounds grid (thumbnails, animated ones show a ▶ corner), Choose…, Remove.
  - Effect (stills) · Animate effect · **Play animated backgrounds**: Always / Home only / Off.
  - **Show behind chats** on or off · **Chat blur** slider · **Chat dim** slider (see 11c).
  - Settings keys: `wallpaper_kind`, `wallpaper_playback`, `wallpaper_chat`,
    `wallpaper_chat_blur`, `wallpaper_chat_dim`, `wallpaper_recent` (JSON list).

**11c. Chat backdrop: the "Zeron frost"** (fixes "way too blurry, not neat")
- The problem: blur 30 with a 0.60 tint leaves half-dissolved shapes, a smudged
  photo behind text. It reads as blur, not as a material.
- The fix: **blur much stronger, until only colour fields remain, then a heavier,
  contrast-checked scrim**, which is what Zeron's window glass looks like:
  - **Bake once**: downscale to 1/8, Gaussian σ ≈ 10 at that scale (≈ 80 device px
    equivalent), upscale bilinear, add 1.5 % blue-noise grain so gradients don't band.
    One still per wallpaper and size; per-frame for animated media at 1/8.
    Replaces the live `ui::frosted(0, 30, …)` pass: sharper-looking *material*,
    and cheaper (no per-frame backdrop blur over the full column).
  - **Scrim** `canvas` at **0.78 by default** (dark) or 0.82 (light), then raised as needed by
    a port of Zeron's `contrast_checked_tint_alpha` so body text keeps ≥ 7 : 1
    against the brightest region of the baked art.
  - Sidebar and panes over art: the same baked frost, scrim plus `SIDE_ART_TINT`.
  - Transcript bubbles and tool cards keep `card` fill at 0.85 so they stay crisp on the material.
  - The composer, popovers and palette use live `float` blur 16 (`MENU_BLUR`) over the baked
    frost: crisp edges, same as Zeron.
- Sliders: Blur (Soft 40 · **Frost 80** · Deep 120) and Dim (0.65–0.90, never below
  the contrast floor). Default is the Zeron frost.
- Tests: snapshot the baked frost for 3 reference images; assert the contrast floor; assert
  no live blur element in the thread tree when art is baked.

### Phase 12 — Motion, accessibility and polish
- Motion (`motion.rs`): pane open and close 180 ms ease-out with the column reflowing in sync;
  popovers 120 ms fade and 4 px rise; hover 80 ms; mascot idle. All gated by reduced motion.
- Keyboard: every control is reachable; visible focus = `hairline_strong` plus a 1 px accent
  inner ring only on keyboard focus.
- Contrast: muted text ≥ 4.5 : 1 on every plane, including over the frost.
- Light theme: a full pass on every frame.
- Windows-first QA (PrintWindow screenshots in a scratch `DOVE_HOME`, per memory), then macOS.

---

## 4. Order, ownership, size

| # | Work | Depends on | Size | Who |
|---|---|---|---|---|
| 0 | Paper frames and tokens | — | M | Opus (design) |
| 1 | Tokens and theme | 0 | M | Opus; Haiku sweep for grey literals |
| 11c | Chat frost bake plus scrim | 1 | S–M | Opus (**ship first: the most visible fix**) |
| 9 | Kit pieces | 1 | M | Opus |
| 2 | Title bar and pane toggles | 9 | M | Opus |
| 3 | Sidebar | 9 | L | Opus |
| 5 | Composer plus chips | 9 | M | Opus |
| 4 | Home | 3, 5 | M | Opus; sprite by hand |
| 7 | Pane host, placement, Files, Details | 2, 9 | L | Opus |
| 8 | Palette | 9 | M | Opus |
| 6 | Transcript | 1 | M | Opus (coordinate with v2 chat-speed work) |
| 11b | Animated wallpapers | 11c | L | Opus pipeline; Haiku for settings plumbing |
| 10 | Settings and pages | 9 | L | Haiku per page, Opus review |
| 12 | Motion, accessibility, QA | all | M | Opus plus one GUI QA driver at a time |

These can run in parallel once Phase 9 lands: **3 (sidebar)**, **7 (panes)**,
**8 (palette)** and **11b (animated media)** touch separate files. Each needs its own
`CARGO_TARGET_DIR` (shared checkout).

## 5. Blockers and notes
- `apps/dove/Cargo.toml` is in an unresolved merge state (`UU`); resolve it before any build.
- The notch files have uncommitted edits on `feat/notch-v2`; land the revamp on its own
  branch (`feat/ui-v4`) cut after the notch work is committed.
- Originality guardrails: our own mark and sprites, Solar icons, Keplorayx accent, our own copy
  ("Start something…", "Needs you", "Pick a file to preview"), and different panel
  composition (Details includes Team and Integrations). Don't reuse the reference
  app's strings, mascot or icons.

## 6. Done when
- Every Paper frame has a matching screenshot in `docs/revamp-v4/shots/`, side by side.
- The chat backdrop shows colour only, with no recognisable shapes, at default settings; contrast tests pass.
- A GIF, an animated WebP and an MP4 wallpaper play on Home, show pre-blurred behind
  chats, pause when unfocused, and stay within the 64 MB and 1 % CPU idle budget (`perf_trace`).
- No ad-hoc grey literals left outside `theme.rs`.
