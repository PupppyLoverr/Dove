# What Zeron's zgx app teaches, and what Keplorayx does with it

A read-only study of `C:\dev\zeron\apps\zgx`, done before any Orbit design work. We take its **mechanisms** (Zeron is MIT, and ports are credited in `THIRD_PARTY_NOTICES.md`). We do not take its look, palette, fonts, art, copy or component recipes.

Paths below are relative to `C:\dev\zeron\apps\zgx`.

## How it gets its polish: the mechanisms

### 1. Values live in the theme. Screens never hold one.
- **What Zeron does:** standard tokens (`$text.secondary`, `$space.3`, `$radius.lg`) come from `themes/*.json`, and app tokens are declared *in terms of* standard ones (`ui/ds/tokens.luau`, 713 lines), so any theme styles them. Each entry starts with `ui.theme.strict("warn")`, and `ZGX_DESIGN_LINT=error` fails on raw colours, spacing, radii, type sizes and shadows. `ui.raw(v, "why")` is the greppable escape hatch (README "The design system's rules" 1–4).
- **Keplorayx:** the same three layers under `ui/kx/`, with tokens in the `kx.*` namespace (never `zeron.*`). Strict mode is on from the first screen, and CI runs the lint at `error`.

### 2. Spacing is a scale, with two half-steps.
- **What Zeron does:** `space.1`–`space.8` = 4, 8, 12, 16, 24, 32, 48, 64, plus two half-steps for controls (2 and 10). Fixed geometry, such as title-bar height, is a named constant, not a token.
- **Keplorayx:** the same discipline. The scale itself is chosen in our token sheet. Our "lens" rhythm may prefer a 6-based half-step, and that is decided in the gallery.

### 3. Corners are concentric, computed rather than chosen.
- **What Zeron does:** `ui/ds/control.luau` `M.nest(outer, ...insets)` gives the inner radius as the outer radius less each inset, where a pill stays a pill and an inset past the radius gives square. `M.contain(props, {radius, inset, border})` rewrites every radius token inside a box, so nested Buttons come out concentric in *all states* without being told.
- **Keplorayx:** port the arithmetic (credited) into `ui/kx/shape.luau` for the "lens" radius family. Add a gallery page that shows every component at rest, hovered, pressed, focused and selected inside each container radius.

### 4. One wash for hover, press and selection.
- **What Zeron does:** one wash covers hover, press and selection. Selection adds only a ring, never a shadow (README just after rule 7).
- **Keplorayx:** the same rule. Our wash is a light lift on the glass (depth from light), and the ring is the agent's own accent where an agent owns the row.

### 5. A container's inset is even.
- **What Zeron does:** a container takes one inset value on all four sides, never separate `px`/`py` (rule 6).
- **Keplorayx:** the same, enforced by `contain`.

### 6. One reduce-motion check, with moves sized by distance.
- **What Zeron does:** `ui/ds/motion.luau` keeps `M.reduce` (a persisted setting) and `M.reduced()`, tracked and OR'd with the system preference, plus `reduced_now()` for handlers. `M.tween` and `M.spring` collapse to the target value when motion is reduced, and fades become a 120 ms crossfade. Column moves are 180 ms, scaling by √distance up to 240 ms, so a wide panel moves at the sidebar's pace. A moving column keeps its content at full width and its edge clips it, so nothing lays out mid-move.
- **Keplorayx:** `ui/kx/motion.luau` with the same single check. Our curve differs: Orbit's eased *overshoot settling* runs on **one shared clock** (the "breath"), and every live indicator reads that one clock. The √distance duration rule and the clip-not-relayout rule are adopted as they are.

### 7. Material and transparency follow the system.
- **What Zeron does:** `ui/ds/material.luau` combines `material.frosted` (the user's setting) with `material.see_through()`. It turns solid when the system asks, using macOS Reduce Transparency, or Windows' Transparency effects through `src/transparency.rs`, which Keplorayx already ports. Popup glass is solid on Windows because zgx popups have no backdrop there.
- **Keplorayx:** the same gate. It is already started, since `system.reduced_transparency` is wired in `apps/keplorayx/src/transparency.rs`. Popups on Windows are drawn solid until that `zgx-asks` item is fixed upstream.

### 8. Data changes in place, never by rebuilding.
- **What Zeron does:** lists are keyed `ui.each`, and changing values are read through functions or states bound to the prop that shows them. `ui.presence`/`ui.switch` are only for real swaps, keyed by *kind*, never by data. `ui/tests/*_in_place.test.luau` (chat, composer, files, lanes…) assert **node identity** across data changes, because lint can't catch this. On the host side, lists publish per entry (`set_key`), never the whole list (README "Patterns").
- **Keplorayx:**
  - Adopted wholesale.
  - `apps/keplorayx/src/host.rs` currently republishes `kx.entries` whole on `Change::Entries`. That fits Phase A, but Phase C moves it to per-entry `set_key`, and the streaming draft is already separate.
  - Every screen gets an `_in_place` test.

### 9. A reload changes nothing.
- **What Zeron does:** `tests/reload.rs` mounts each window headless with host actions stubbed, settles it, reloads, and checks every node's box, both immediately and over the following frames. It also checks that the reload asked the host for no data again. Motion and engine calls must sit behind keyed state (`visits.once`, `ui.state`).
- **Keplorayx:** port the harness as `apps/keplorayx/tests/reload.rs` once there is more than one screen (Phase B).

### 10. No polling, no UI-thread I/O, timers owned by their effect.
- **What Zeron does:** waits on events (`zgx::task::latest`, watches, poll on a wake pipe). Timers exist only for a deadline someone sees, and `ui.after`/`ui.every` are created in effects so a rerun cancels them.
- **Keplorayx:** already followed in `host.rs`, which uses `recv_async` with no polling and reads the store on a thread. The rule goes into `apps/keplorayx/README.md` and holds idle frames at 0.

### 11. A workbench, fixtures and stubs.
- **What Zeron does:**
  - `ui/gallery.luau` (2,101 lines) shows every component in both themes side by side, and is edited live.
  - Around 80 `*_fixture.luau` files and `*_stub.luau` stores (`settings_stub.luau`, `cloud_stub.luau`) let each screen run headless without the engine.
  - `ZERON_ZGX_STEP=<step>` pins an onboarding step across saves.
  - `--inspect` plus `zgx inspect tree/snapshot/click` lets agents drive it.
- **Keplorayx:** the same shape: `ui/gallery.luau`, `ui/fixtures/*.luau`, one stub per store in `ui/kx/`, `KX_STEP=<step>`, and `--inspect` (already wired in `main.rs`). Audit PNGs go to `apps/keplorayx/docs/design/`.

### 12. Onboarding: one module per step, shared state, shared actions.
- **What Zeron does:** `ui/screens/onboarding/` has `init`, `state`, `actions`, and one file per step (`signin`, `username`, `card`, `org`, `providers`, `github`, `invite`, `appearance`, `done`). Order lives in `ui/onboarding_order.luau`, and `ui/ds/onboarding_frame.luau` is the one frame they all sit in.
- **Keplorayx:** First Light uses the same structure with our own steps: welcome, agents, project, team, notch, privacy. The privacy step carries an **optional, skippable** Keplorayx account card: the existing Woozlit sign-in, kept unchanged (see the auth study). The "bring / fresh" sync offer follows only if the user signs in. Provider sign-ins appear inside "Your agents".

### 13. The hidden-window hand-off.
`src/main.rs` `intro()` (350–445) plus `intro_window()` (1045). The pieces:

| Piece | Where | What it does |
|---|---|---|
| The overlay | `intro_window()` | Transparent, undecorated, `covers_display`, `AlwaysOnTop`, no shadow, with pointer passthrough on until the script asks for `interactive`. |
| `prepare {x,y,w,h}` | `intro()` | Opens setup's window *invisible* at that rect, converting points to physical pixels with the overlay's scale. Its script runs and draws its first frame meanwhile. It also reports its corner radius (`onboarding.radius`) so the intro's shape lands on it exactly. |
| `reveal(at)` | `intro()` | Shows the window underneath the still-opaque scene and passes the click time as `onboarding.epoch`. Both copies' drift then reckons from the same moment, so the pixels match. |
| `done` | `intro()` | Sets setup's `onboarding.arrived = true` (its frame stops holding opaque and clears to glass), focuses it, and closes the overlay. |
| The transition | `ui/ds/decode.luau` | Runs on a **still snapshot**, never the live scene, to avoid an offscreen layer every frame. The overlay is capped at 60 fps. |

**Keplorayx:** write the same four-beat protocol fresh (`prepare`, `reveal`, `arrived`, `done`) in `apps/keplorayx/src/intro.rs`. Our transition is our own: the orbit of light tightens into the window's rounded rect, and the window clears to glass on the same frame. It keeps Zeron's two hard rules: animate a still picture, not the live scene, and share one epoch so there are no seams. There is no character-decode effect and no card wall.

## What Keplorayx does differently

| Area | Zeron | Keplorayx |
|---|---|---|
| Identity | Its palette, Geist / Instrument Serif, the member card and sky art | "Orbit": night-sky neutrals, one luminous accent per agent, a warm attention colour spent only when something needs you, our own fonts under `fonts/` |
| Motion | CSS `ease`, distance-scaled | Eased overshoot that settles, on one shared "breath" clock. Idle frames = 0. |
| Intro | Character decode of a sunrise scene | First Light: a point of light draws its orbit, then tightens into the window |
| Onboarding | An account-first flow (sign-in, username, card, org, invites) | Local-first. The Woozlit account is optional and skippable, provider sign-ins are inline, and privacy is in plain words. |
| Engine | A separate engine process over IPC | Dove's engine in process (`dove_core::Engine`) |
| Overview | Lanes | Constellation view |
| Composer | Composer plus palette | The Ray bar: one bar for compose, Ctrl+K, `>`, `@` and `/` |

## Checklist carried into Phase B

1. `ui/kx/tokens.luau` and `themes/*.json` with strict mode and design lint at `error` in CI.
2. `kx/shape.luau` (nest/contain), `kx/motion.luau` (one reduce check, one clock) and `kx/material.luau` (the frosted/solid gate).
3. Gallery: every component × every state × both themes, plus a reduced-motion toggle.
4. A stub per store, a fixture per screen, and an `_in_place` test per list screen.
5. A port of `tests/reload.rs`.
6. Audit-shot PNGs after each pass.
7. Intro: `prepare`/`reveal`/`done` with a shared epoch, animating a still picture at 60 fps.
