# Themes

A theme is the colours Zeron's window wears. It's only colours, so it works with any template, and it can't run anything. The person picks one for light and one for dark (Settings → Themes, or Settings → Appearance). Agents read, make, change and switch themes with Zeron's MCP tools; this guide is the whole reference.

## The authored set

The window's own themes are written in a smaller form than a palette: **42
tokens in nine groups** (`ui/ds/themes/keplorayx-{dark,light}.json`). Everything
else a theme used to name — `hover`, `pressed`, `selected`, `focus.ring`,
`scrollbar.thumb`, the shadows, `ink.disabled`, the tone washes, `radius.*`,
`font.size.*`, `font.line.*`, `font.weight.*`, `space.1`…`space.8`,
`density.*`, `measure.*`, `duration.*` — is **derived** from them, in
`ui/ds/authored.luau`, one line each.

A theme file that sets a derived token is **rejected, and says which**:

```
stage 3: hover: "hover" is derived (ink.primary at 0.05) and not a theme's
to set. Remove it; the window computes it.
```

That is the point of the cut. A theme of 267 leaves can disappear a selected
row by setting its fill to nothing; a theme of 42 can only choose bad colours,
because the things that must stay visible are computed from what it chose.

| Group | Tokens |
|---|---|
| `surface` | `window`, `base`, `raised`, `overlay` |
| `ink` | `primary`, `secondary`, `tertiary` |
| `line` | `hairline`, `strong` |
| `accent` | `accent`, `accent.ink` (the words on the accent) |
| `tone` | `ok`, `warn`, `danger`, `info` |
| `attention` | `attention`, `attention.ink` — the warm colour, and only for what needs you |
| `agent` | `agent.1` … `agent.8`, eight fixed ids |
| `font` | `sans`, `mono`, `display`, `size`, `line`, `weight`, `strong` |
| `metric` | `unit`, `radius`, `row`, `control`, `gutter`, `conversation`, `page`, `duration`, `ease`, `blur` |

The ranges that keep a theme from breaking the window (a clamp moves the value
and says so, rather than rejecting): `font.size` 11–18, `font.line` ≥ 1.2 × the
size, `font.weight` 300–500 with `font.strong` above it and at most 700,
`metric.unit` 2–8, `metric.radius` 0–16, `metric.row` 24–48 (the hit target's
floor), `metric.control` 20–40, `metric.gutter` 8–32, `metric.conversation`
480–1200, `metric.page` 480–800, `metric.duration` 0–400, `metric.blur` 0–64.

## The check

Seven stages, in order, run by `src/authored.rs` (1–5) and `src/layout_smoke.rs`
(6) before a window wears a theme:

| # | Stage | A failure |
|---|---|---|
| 1 | the file parses as JSON | reject |
| 2 | its shape: `name`, `type`, `tokens`; no other field | reject |
| 3 | every key is authored, of its kind, in range | reject (clamps adjust and warn) |
| 4 | its `{ref}`s resolve, no cycles, no derived token named | reject |
| 5 | the inks read: 4.5:1 for text, 3:1 for the faint and the tones | lift and warn, reject only where no colour can read |
| 6 | **the layout holds**: the real shell mounted at 1024/1280/1600, every label measured against its own type and its row | reject, naming the surface |
| 7 | `zgx lint` over a template's screens | templates only |

Stage 6 is the one a clamp cannot do. A 24-point row is a legal hit target and
an 18-point body is a legal body size; together, a row's second line is cut, and
only a layout pass sees it. `cargo test --bin keplorayx layout_smoke` proves
both directions: the shipped themes hold at all three widths, and a theme set
to (size 18, row 24) is caught.

Whoever imports a theme through Settings → Themes runs stages 1–6 before it is
installed or listed; `themes.check { path }` runs the same check writing
nothing, which is the tool an agent uses before asking the window to wear
something new.

## The tools

- **`list_themes`**: every theme, with its `id`, `name`, where it comes from (`built in`, `a template's`, `yours`), whether you can change it (`editable`), its variant for each scheme (`variants: { light?, dark? }`) and which schemes it's worn for now (`in_use`). `wearing` says what each scheme wears now.
- **`get_theme { id }`**: a theme's colours, a palette for each scheme it has, in the shape the next two take. `adjusted` lists what the window moves to keep it legible.
- **`create_theme { name, light?, dark? }`**: a new theme of yours from palettes (one at least). Or **`create_theme { name?, vscode }`** from a VS Code colour theme's JSON (`colors`, `tokenColors`, `type`), through the importer Settings → Themes uses.
- **`update_theme { id, name?, light?, dark? }`**: change one of yours. A palette replaces that scheme's; `null` takes it away. Its ids stay the same, so while it's worn the window restyles at once.
- **`delete_theme { id }`**: yours only. A scheme it was worn for goes back to the template's theme, or to Zeron's.
- **`use_theme { id, slot? }`**: wear it. `slot` is `"light"`, `"dark"` or `"both"` (the default: each scheme it has).

Built-in themes and a template's own theme are read-only. To change one, copy it: `get_theme`, edit the colours, then `create_theme` with a new name.

"Make me a theme and switch to it" is two calls: `create_theme`, then `use_theme` with the `id` it returns.

## A palette

One scheme's colours. Each is `#rrggbb` (or `#rrggbbaa`).

| Field | What it colours |
|---|---|
| `background` | the window |
| `raised` | raised things: a chip, a selected row |
| `text`, `muted`, `faint` | body text, quieter text, the faintest text |
| `accent` | the accent: buttons, links, the focus ring |
| `danger`, `warning`, `success` | the tones |
| `syntax` | 12 code colours, in order: comment, keyword, string, number, type, function, property, variable, punctuation, tag, attribute, invalid |
| `shell`, `card` | optional: the sidebar and title bar, and a card (the window's colour where absent) |
| `terminal`, `ansi` | optional: the terminal's background and its 16 colours (black, red, green, yellow, blue, magenta, cyan, white, then the bright ones) |

A minimal dark palette:

```json
{
  "background": "#14181f", "raised": "#232a35",
  "text": "#d8dee9", "muted": "#a3acb9", "faint": "#7c8594",
  "accent": "#7aa2f7", "danger": "#f7768e", "warning": "#e0af68", "success": "#9ece6a",
  "syntax": ["#5c6773", "#c792ea", "#c3e88d", "#f78c6c", "#ffcb6b", "#82aaff",
             "#d8dee9", "#d8dee9", "#a3acb9", "#f7768e", "#ffcb6b", "#f7768e"]
}
```

So `create_theme { "name": "Harbor", "dark": { …that… } }` makes a dark-only theme. Give `light` too for both.

## Legibility

The window keeps every theme readable. Text needs 4.5:1 against every surface, and the faintest text 3:1. Fills need 3:1 on the window, and each tone's words 4.5:1. Code colours need 4.5:1, with comments and punctuation at 3:1. An ink that's too faint is moved toward black or white until it reads. Your colours are kept as you wrote them; `adjusted` (`{ scheme, role, from, to, min }`) says what the window wears instead. If you'd rather choose, change those colours and save again.

## Where it's kept

This device's: `theme-library.json` (your themes) and `theme-choice.json` (what each scheme wears) in Zeron's data folder. Another of your devices has its own. A template's own theme is its `theme.luau` (docs/templates.md, Colours and themes).
