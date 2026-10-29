# Consolidation plan: every worktree into the main folder

Written 2026-10-29, refined the same day. Nothing below has been run yet.

**Decided:** the app is renamed **Keplorayx**, short **K22**. Every line of work lands on one branch, and the rename is finished on that branch after the merges, so the merges don't fight it.

## Where everything is today

| Place | Branch | What is there | Already in `feat/notch-v2`? |
|---|---|---|---|
| `Downloads\dove` (main folder) | `feat/notch-v2` @ `61908ce` | Notch V11, black island with opt-in glass, computer use v2 (merged), **Host layer: WSL / SSH / Cua Spaces, Roadmap v2** | — |
| main folder, **uncommitted** | — | (a) **Keplorayx rebrand, first pass + new app icon** (~30 strings, exe metadata, installers); (b) **OpenCode inside a Space / over SSH**: port forwarding, `OPENCODE_*` env, `dove.remoteDirectory` patch to vendored OpenCode, smoke example, ROADMAP §11; (c) `docs/notch-v3-plan.md`, `docs/ui-revamp-v4.md`; (d) stray `target-check/` build folder | No |
| `Downloads\dove-cu` | `feat/computer-use-v2` @ `db5b76e` | Computer use: pixel frames, markdown tree fallback, serve example | **Yes, fully** |
| `Downloads\dove-ui-v4` | `feat/ui-v4` @ `f5674e2` | 11 commits: animated wallpapers, K22 icons + Inter/JetBrains Mono, settings cards, **UI v5 sidebar (as in the screenshot)**, K22 v5 icon pass, dock shares sidebar material, calmer chat. Untracked `k22-evidence/` (QA screenshots) | No (branched at `7f54b2a`, 11 behind) |
| `Downloads\dove-notch-v3` | `feat/notch-v3` @ `545bb05` | First 8 ui-v4 commits, plus **Notch V3: talk to K22 (call mode), plan review, live views** and **black-and-white notch + minimal call** | No (branched at `5f03eb3`, 1 behind) |
| `C:\dvw\target-ui-v4` | — | Warmest cargo target (debug + release) | — |

Stray one-commit branches (26 Oct snapshots):
- `feat/flock` `12f2ba7`: trims README, adds `target-release.log`. **Drop.** Its real fix `98cb6b8` is in everything.
- `recovered/feat-team-ui` `b330f69`, `recovered/feat-superapp` `20c2e1c`: WIP from the worktree cleanup. **Review hunk by hunk**; cherry-pick only what is truly missing, with your OK.

Session → work:
- *Dove market positioning strategy* → Host layer + uncommitted OpenCode-in-Space work.
- *Worktrees cleanup and sync* → `feat/notch-v3`.
- *UI/UX Revamp* and its side chats (icon swap, rebrand) → `feat/ui-v4` + uncommitted rebrand.
- *Screen recording analysis* → Flock terminal fix `98cb6b8`, already merged.

## Rename scope

About 150 string literals in `apps/dove/src` still say "Dove", and ui-v4 and notch-v3 each bring ~5 more. The rename is split into tiers by risk:

| Tier | What | When |
|---|---|---|
| **T1: what a person sees in the desktop app** | Window title, tray, menus, settings, toasts, errors shown in the UI, installer, exe metadata, icon. ~30 done. | This consolidation (step 6) |
| **T2: what agents read** | Tool descriptions and errors returned to agents ("Dove's in-app browser", "Dove → Settings → Plugins"), the text of the `dove-browser` / `dove-computer` / `dove-memory` / `teach` skills, the agent guides. Agents repeat this to the user, so it has to match. | This consolidation (step 6) |
| **T3: companion surfaces** | Chrome extension (`"Dove for Chrome"` → `"Keplorayx for Chrome"`), its pairing messages, Android `app_name`, iOS, landing page, README, docs. | Same branch, right after T1/T2 (step 7) |
| **T4: identifiers** | `dove.exe` → `keplorayx.exe`, `~/.dove` data folder, `DOVE_*` env vars, app ID `dev.dove.Dove`, the `dove` MCP server name (`mcp__dove__*`), skill folder names, crate names, repo name. Renaming these breaks existing installs, saved settings, paired extensions and agent configs unless there is a migration. | **Not in this consolidation.** Separate branch with a migration (read the old names, write the new ones). Needs your call on which to rename. |

Rules for T1–T3:
- "Keplorayx" in full in sentences; "K22" where it already appears in design (the notch's K22 dot, "talk to K22", icon set name). Don't swap one for the other.
- Leave comments, log targets, internal markers the code matches (e.g. the `(Dove)` tab tag in `browser/control.rs` and its test, until the extension sends the new one, so change both together in T3), and anything in T4.
- Folder names a user sees on disk (e.g. `composer/mod.rs:747` creates `…\Dove`) count as T4: they need a migration, so leave them.

## Steps

### 0. Freeze and back up
1. Make sure no session is editing the main folder or the three worktrees. All four named sessions are idle now; don't start new ones there until step 8.
2. Tag every tip: `backup/2026-10-29/{notch-v2,notch-v3,ui-v4,computer-use-v2,flock,recovered-team-ui,recovered-superapp}`.
3. Snapshot the dirty main folder without touching it: `git stash create` → `git branch backup/2026-10-29/main-dirty <sha>` (it covers tracked files; copy the 2 new icon files and 2 docs separately).
4. Move `dove-ui-v4\k22-evidence\` out of the worktree (to `docs\qa\k22-evidence\` if you want it committed, otherwise outside the repo).

### 1. Commit the main folder's uncommitted work (on `feat/notch-v2`)
Split by owner, so each can be reverted alone. Check each file's hunks with `git diff` first, since `settings.rs` and a few others may carry both.
- **A — Hosts: OpenCode runs inside a Space or over SSH.** `crates/dove-harness/src/{host,external}.rs`, `examples/smoke.rs`, `vendor/zeron-harness/src/opencode/mod.rs`, `docs/ROADMAP.md`.
- **B — Rename the app to Keplorayx (K22), with its new icon.** The string edits, `build.rs`, `Cargo.toml` description, `packaging/*`, icon adds and deletes.
- **C — Docs:** `notch-v3-plan.md`, `ui-revamp-v4.md`, this plan.
- Add `target-check/` to `.gitignore` and delete it (stray build folder).
- Gate: `cargo check -p dove` in the warm target.

### 2. Create the integration branch
`git switch -c integrate/k22` from the new `feat/notch-v2` tip.

### 3. Merge `feat/ui-v4`
Expected conflicts: `composer/mod.rs`, `main.rs`, `notch/{mod,tokens,views,views/canvas,views/menus,views/pages}.rs`, `settings.rs`, `shell.rs`, `transcript.rs`, `README.md`, `THIRD_PARTY_NOTICES.md`, and every file commit B touched (`assets.rs`, `island/mod.rs`, …).
- **Sidebar, shell, transcript, dock, settings cards, icons, fonts:** ui-v4 wins (that's the screenshot).
- **Notch files:** notch-v2's structure (opt-in glass, black island) with ui-v4's icon names and fonts. Keep this light; step 4 rewrites them.
- **Composer:** notch-v2's device chip (This PC / WSL / SSH / Spaces) *and* ui-v4's visuals.
- **Strings:** "Keplorayx" wherever either side has user-facing text.
- Gate: `cargo check -p dove`, then commit the merge.

### 4. Merge `feat/notch-v3`
Only Notch V3 (`ef15111`) and black-and-white (`545bb05`) are new; the rest arrived with ui-v4.
- Expected conflicts: icon SVGs and Android drawables both branches added after `689e34d` (add/add), `settings.rs`, `shell.rs`, `transcript.rs`, `main.rs`, notch files.
- **Icons:** ui-v4's K22 v5 set wins; keep any glyph only notch-v3 uses (grep the `notch/` icon names against the v5 set).
- **Notch:** notch-v3 wins, then check the opt-in glass switch from notch-v2 still works.
- Gate: `cargo check -p dove`, commit the merge.

### 5. Confirm computer use and the stray branches
- `git merge-base --is-ancestor feat/computer-use-v2 integrate/k22` must succeed.
- Diff each `recovered/*` commit against `integrate/k22` and list hunks that are missing. Ask before cherry-picking. Drop `feat/flock`'s WIP commit.

### 6. Rename, T1 and T2 (desktop app + agent-facing text)
One commit. A Haiku subagent does the mechanical sweep from a list I prepare: every `"…Dove…"` literal in `apps/dove/src`, `crates/*/src` and `skills/*` classified as T1/T2 (change) or T4/internal (leave). I review the diff.
- Gate: `cargo test --workspace` (some tests assert on these strings), then `grep` shows no user- or agent-facing "Dove" left outside the T4 list.

### 7. Rename, T3 (companion surfaces)
One commit per surface: Chrome extension (manifest name and description, pairing messages, and the `(Dove)` tab tag together with `browser/control.rs` and its test), Android `app_name`, iOS, landing, README + docs.
- The extension keeps its ID, so a paired browser stays paired.

### 8. Verify (warm target `C:\dvw\target-ui-v4`, never a fresh one)
- `cargo fmt --check`, `cargo clippy --workspace`, `cargo test --workspace`, release build.
- Launch with a scratch `DOVE_HOME`, one GUI driver at a time:
  - sidebar matches the screenshot; K22 v5 icons on every page; wallpaper + dock material;
  - notch: black-and-white capsule, Agents tab, plan review, live views, **K22 call** start / mute / end;
  - computer use: a capture and a click through `computer`;
  - composer device chip lists This PC / WSL / SSH hosts / Cua Spaces; one remote run with `smoke.rs` and `DOVE_HOST`;
  - window title, tray, exe icon, installer name say Keplorayx; the Chrome extension pairs and shows Keplorayx.

### 9. Land and clean up (each needs your OK)
- Main folder stays on `integrate/k22`, or `feat/notch-v2` fast-forwards to it.
- Push and open a PR `integrate/k22 → main`.
- Remove the three worktrees with `git worktree remove`; keep their branches and the backup tags.
- Then T4, on its own branch, once you decide which identifiers to rename.

## Risks
- **Notch files** change in all three lines of work; most of the time goes there.
- **Icon set:** two branches created the same SVG names independently. A wrong pick shows as a missing or odd glyph, so QA every page, not just Home.
- **Rename vs tests:** tests and the extension match some strings exactly; T3 changes those pairs together.
- **Shared checkout:** a session that writes to the main folder during steps 1–8 can lose work or break the merge.
- **Build time:** release builds took 2+ hours when several ran at once. Run one build at a time.
