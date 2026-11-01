# Zeron parity plan

What to take from Zeron next, in order. Compared against `C:\Users\ravth\Downloads\zeron` at `55ffd414`
(`crates/ui/src`). Sizes and tints below are read from its source, not estimated.

Already taken: the title bar, right dock and explorer column, the window glass, the sign-in and sync pages.

## Status

Done: the info card (one frosted card, docking beside the chat, lanes and ports rows), the centred input bar's numbers
and selectors, smooth scrolling, per-file tabs in the dock.

Not done, in the order below: the plate morph into a thread, reveal motion on the info card, project run actions, the
transcript fade, background processes, the image viewer and `show_image`, appshots, sidebar sections, settings search
reveal, sounds.

## 1. The info bar (Zeron's session card)

Same style and tint, Keplorayx's own content. Today `render_info_card` in `apps/dove/src/shell.rs` is a stack of
four separately bordered, foldable cards (Environment, Context, Changes, Actions) at 300 px with `shadow_md`.

| | Zeron (`shell/session_info.rs`) | Keplorayx now | Change |
| --- | --- | --- | --- |
| Width | 272 | 300 | 272 |
| Position | 12 inset from the column's right edge | 14 inset, floating only | 12 inset |
| Beside the chat | Docks when the column has room, and the transcript and composer shift left by just enough (`session_card_reserve`) | Always floats over the transcript | Port the reserve |
| No room | Floats over the transcript and dismisses on click-outside | Same, without the dismiss | Add the dismiss |
| Surface | One `popover::surface_bg` card, `frost::MENU_BLUR`, 1 px `theme.border`, shadow only when not frosted | Four cards, `glass_overlay`, always a shadow | One frosted card, plus optional lane and port cards under it (8 gap) |
| Rows | `PICKER_ROW_HEIGHT`, 15 px glyph, 8 gap, 13 text, not foldable | 30 px rows, 15 px glyph, 10 gap, foldable with chevrons | Zeron's rows, no folding |
| Motion | Reveal 0 to 1 on the shell's resize clock; snaps on a chat switch | Appears instantly | Port the reveal |

Content, keeping what Keplorayx already shows:

1. **Where it runs:** device, project, branch. The branch row gets Zeron's `+N −M` totals, which open the branch diff.
2. **Context and usage:** one quiet line with the meter, not a card. Zeron keeps these under the composer, so it is a
   judgement call (see the questions below).
3. **Run actions:** Zeron's project actions (`project_actions.rs`) are new to Keplorayx.
4. **Lanes card:** running subagents and side chats (`session_lanes.rs`), with a running count on the toggle
   (`running_pill::mark_titlebar_button`).
5. **Ports card:** servers the thread's agents and terminals started. Keplorayx only scans terminal text for URLs today.
6. **Thread actions** (side chat, fork, open folder, Chat / Terminal, copy path) move to a small row at the bottom.

## 2. The centred input bar

`render_body`'s `hero` branch in `apps/dove/src/composer/mod.rs`, against Zeron's `composer.rs`.

| | Zeron | Keplorayx now | Change |
| --- | --- | --- | --- |
| Corner | `COMPOSER_RADIUS` 26 | `design::radius::COMPOSER` | Confirm it is 26 |
| Width | `COMPOSER_MAX_WIDTH` 768 | Wider on Home (about 755 logical at 1176, check at large windows) | Cap at 768 |
| Text area | 76 to 260 tall, line height 22.75 | `min_h` 120 for the whole card, text 64+ | Zeron's numbers |
| Actions row | 2 + 32 + bottom pad, inside the plate: attach left, then model, send | Same shape, plus mode and voice in the row | Keep the shape; mode moves down |
| Above the plate | Target selectors (project, device), 20 px, right-aligned | None | Add |
| Below the plate | Git selectors (checkout, branch), 20 px | Project and device chips | Move project and device up; put checkout and branch here |
| Fill | `composer_surface_bg`: dark `composer_sidebar_tint`, 15% coverage | `composer_hero_bg` | Use the same tint |
| Edge | `composer_surface_border`: `hsla(210, .18, .78, .09)` on dark frost | `composer_border` | Compare and match |
| To a thread | `FlipMorph::new_thread_transition` morphs the plate into the docked bar | Cut | Port last, it is the largest piece |

## 3. Anything else worth taking

Checked against what Keplorayx has today. Highest value first.

| Feature | Zeron | Keplorayx | Why |
| --- | --- | --- | --- |
| Smooth scrolling and overscroll | `smooth_scroll.rs`, `overscroll.rs` | Missing | The biggest "feels fast" gain per line of code; wheel notches glide |
| Per-file tabs in the dock | `RightSurface::File(id)` | One shared Files tab | Gap left by the dock port |
| Background processes | `background_tray.rs`, `docs/background-processes.md` | Missing | See, read and stop what an agent left running; helps RAM and CPU directly |
| Transcript fade under the title bar | `edge_fade.rs`, content row spans the window | Hard clip at the bar | Finishes the glass look |
| Image viewer | `image_viewer.rs`, `tool_images.rs`, `reply_images.rs` | Missing | Agents' screenshots open properly |
| `show_image` for agents | PR #1036 | Missing | Goes with the viewer |
| Project run actions | `project_actions.rs` | Missing | One click to run the app; feeds the ports card |
| Appshots | `appshots.rs` | Missing | Attach a window capture to a prompt |
| Sidebar sections and pins | `sidebar_sections.rs`, `sidebar_pins.rs` | Pins only | Long lists of threads |
| Settings search reveal | `settings/search/reveal.rs` | Search, no scroll-and-flash to the row | Small, polish |
| Sounds | `sound.rs` | Notifications only | Small, polish |

Skip for now: Zeron's Cloud machines (Keplorayx has its own hosts), the mobile rewrites, and the landing site.

### Performance ideas from Zeron

- `docs/performance-resource-usage.md`: quads were its largest cost, and a completed reply kept requesting frames from
  a spring that never parked. Check Keplorayx's scroll follow for the same defect.
- `docs/performance-idle-presence.md`: nothing should repaint while idle.
- `docs/performance-runway-layout.md`, `performance-runway-scroll.md`: long transcripts.
- `docs/performance-composer.md`: typing latency.

## Order

1. Info bar: card and rows, then docking with the reserve, then the lanes and ports cards.
2. Centred input bar: numbers and selectors first, the morph last.
3. Smooth scrolling and the transcript fade.
4. Per-file tabs, then background processes.
5. The rest of the table, as wanted.

Each step ships on its own and is checked against Zeron's source values on a scratch `DOVE_HOME`.

## Questions

- **Context and usage in the info bar:** Zeron's card has neither (they sit under its composer). Keep a quiet line in the
  card, or move them out as Zeron does? The plan keeps the line, so nothing is lost.
- **Mode chip on the centred bar:** Zeron shows access mode under the plate in threads. The plan moves it there on Home as well.
