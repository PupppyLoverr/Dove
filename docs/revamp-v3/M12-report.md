# Revamp v3: M12 report

Branch `revamp/m0` (worktree `C:\dvw\m0`). It holds M0–M6 plus merges of `revamp/m7` to `revamp/m11`, each built in its own worktree, then the M12 fixes. Nothing has been pushed, and `main` is unchanged.

## What M12 did

- Merged M7–M11. Three kinds of conflicts came up:
  - **Duplicate test fixes:** every branch had fixed the same two stale tests. Kept one copy of each.
  - **`home.rs`:** combined M9's hero view with M6's `sidebar_shown()`.
  - **`transcript.rs`:** combined M7's error strip with M10's `has_text` turn ends.
- Moved the Bypass confirmation and the mid-turn Chat/Terminal switch sheet onto the kit modal: scrim, `dialog_in`, Enter and Esc, and a click outside cancels.
- Got `cargo clippy --workspace --all-targets -- -D warnings` clean. This meant removing dead code left by the merges, marking test-only helpers `#[cfg(test)]`, and fixing a few style lints.
- `cargo test --workspace` passes:
  - `dove`: 195 tests.
  - `zeron_harness`: 303 tests.
  - Every other crate is green.
- Release build: `cargo build --release -p dove -p dove-escrow` (fat LTO).
- Ran the perf harness at 1, 5 and 10 sessions, and diffed the inventory against the `4ef1077` baseline.
- Fixed two tooling bugs:
  - **Inventory script:** it cut each file at the first `#[cfg(test)]`, even when that attribute was on a single helper, so everything after the helper went uncounted. It now cuts only at a test module.
  - **Perf seed:** it left `last_run` NULL. The scheduler counts from `last_run`, and the editor always stamps it, so seeded automations never fired. Every earlier run had reported "0 started".

## Feature inventory (baseline `4ef1077` → M12)

The full diff is in `inventory/diff-baseline-to-m12.txt`.

| Area | Before → after | Notes |
|---|---|---|
| Routes | 11 → 11 | none removed |
| Side-panel tabs | 10 → 10 | none removed |
| Settings sections | 13 → 13 | Renamed as approved: Composer → General, Keyboard → Shortcuts, Remote Control → Remote, Crew & Team → Team. Archived became a section. |
| Settings keys | 42 → 48 | none removed |
| Shortcuts / fixed bindings / palette actions | 17 / 8 / 24 → same | none removed |
| UI strings | 1365 → 1536 | Every removed string is reworded copy. Most are the M5 settings rewrite. Thread sort labels now read "Attention / Most recent / Title". The inline "Delete?" step is now a confirmation dialog. |

## Performance (release build, `perf/m12-summary.md`)

The window was visible but not focused, so GPUI caps it at about 30 fps. The PATH was minimal and the fake agent streamed 240 chars/s per session. The sessions ran in the background, not in open tabs.

| Sessions | Start → window | Idle CPU (main) | Streaming CPU (main) | Keplorayx RAM (private) | Harness RAM | Key → frame p50 / p99 |
|---|---|---|---|---|---|---|
| 1 | 384 ms | 0.73% | 21.6% | 95 MB | 2 MB | 56 / 64 ms |
| 5 | 357 ms | 0.62% | 29.3% | 109 MB | 5 MB | 57 / 73 ms |
| 10 | 370 ms | 0.52% | 39.7% | 114 MB | 9 MB | 55 / 72 ms |

The phase-1 baseline was measured on a **debug** build, so the comparison is only rough:
- Start → window was 1.1–2.0 s.
- Idle CPU was 2.8–3.7%, with about one shell render per second forever.
- Main private memory was 102–154 MB.

M12 shows zero idle renders, 0.5–0.7% idle CPU and 114 MB at 10 sessions. That is well inside the 500 MB target.

**Open item: streaming CPU.** Ten background sessions cost about 40% of one core, with no shell renders. That works out to about 2 ms of main-process work per text delta, and about 10 ms per delta for a single session. No streaming baseline exists to compare against: the M0 harness never started sessions, because of the seed bug above. This needs a profiling pass. Likely suspects are per-event transcript work, transcript writes and the M10 index upkeep.

Key → frame latency while streaming (p50 about 55 ms) includes the 30 fps cap on unfocused windows. It was 31–39 ms in an earlier run that never started its sessions, and should be re-measured with `-Activate`.

## Notes and flags

- **Deletes now confirm (M8).** Three deletes that used to be instant now ask first: bulk Delete on Threads, Delete on Archived, and draft Delete on Kanban.
- **Licence (M11).** The "Shades of Purple" theme's licence adds a condition that anything built with it should also be MIT-licensed. Keplorayx is Apache-2.0. Either drop that one theme or accept the condition.
- **Backdrop (M9).** `Shell::render` still runs on each backdrop frame: one window at vsync, with no timer.
- **Not yet checked visually.** M7–M11 and the M12 dialogs have not been checked in the running app. Only build, lint, tests and the perf harness have run.
- **Startup sweep.** Even with a minimal PATH, the daily model sweep (120 s after launch, when the last fetch is over 24 h old) found an installed `droid`, and it printed a device-login prompt in the perf log.
