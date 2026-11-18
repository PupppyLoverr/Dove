# STATUS

Hand-written status was wrong by 40 commits on 2026-11-06 (it claimed "local only, nothing is
pushed" five days after the branch was pushed), so this page now carries only what `git` and a
build can prove. Numbers below are from **2026-11-18** on `zgx/phase-a` and say how to re-derive
them. If they disagree with your checkout, the checkout is right.

```sh
git rev-list --count main..HEAD          # 86 commits, 2026-11-04 … 2026-11-16
git diff --shortstat main...HEAD         # 7,857 files, +2,174,769 / −46,822
git rev-parse HEAD origin/zgx/phase-a    # equal: the branch is pushed, and main is untouched
```

## What runs today

`zr/` is the generation that builds and runs: `keplorayx.exe` (a zgx host running the Luau screens
in `zr/apps/zgx/ui`) talking over loopback to `keplorayx-engine.exe`, built from `zr/apps/zeron`
plus the engine crates in `zr/crates`. It is a **second workspace** (`zr/Cargo.toml`), not a member
of the root one.

```sh
(cd zr/apps/zgx && cargo build --locked)                              # the app
(cd zr && cargo build --locked -p zeron --bin keplorayx-engine)       # the engine
cp zr/target/debug/keplorayx-engine.exe zr/apps/zgx/target/debug/     # it is looked for beside the app
zr/apps/zgx/target/debug/keplorayx.exe main
```

- **Editing `zr/apps/zgx/ui/**` needs no rebuild.** `zr/apps/zgx/src/main.rs:967` watches the
  scripts and reloads them as they are saved, keeping state; only Rust changes need `cargo build`.
- **The other two generations build nothing you can ship.** `apps/dove` (GPUI) is excluded from the
  root workspace and kept as the reference; `apps/keplorayx` is a root-workspace member that never
  ran as the product after Wave 2 landed.
- **`STATUS.md` used to describe `apps/keplorayx`.** It does not; see `plans/rewrite-plan.md` in the
  captain drive for the sequence.

## Verified on 2026-11-18 (Windows 11, i7-10810U, 16 GB)

- **The app builds.** `zr/apps/zgx`, warm target, 19 warnings, 0 errors, **85 s**.
- **The engine builds.** `-p zeron --bin keplorayx-engine`, warm but three days cold, **29 m 33 s**,
  4 warnings, 0 errors. On this laptop budget half an hour for an engine touch.
- **It launches.** `keplorayx.exe` with a fresh data dir starts `keplorayx-engine.exe` on its own
  ("engine: started … (port 27771)", "engine: connected") and opens its 680×500 intro window. The
  intro then closes and **no setup window appears in its place** — the app stays up with no window
  and nothing on stderr. `keplorayx.exe main` against a fresh workspace exits immediately with no
  window and no message, so on a fresh data dir the entry point is `keplorayx.exe`, not `… main`.
- **One warning to chase:** `chat: cursor's models didn't list: harness protocol error: model
  catalog unavailable; retry`.

## What is not verified

- The last four commits (`91ad04c` … `6b5efb8`, 2026-11-15/16) have never been driven by hand. The
  build was the first thing to run them.
- No macOS or Linux build of `zr/` has ever been produced by CI or locally: no workflow builds it,
  and the only machine in this checkout is Windows.
- `.github/workflows/{ci,package}.yml` still build `-p dove`, which is excluded from the workspace,
  so those jobs do not describe this branch's app.
- `zr/scripts/package-{macos,windows}.sh` expect a `zeron` binary and a `zeron-engine` binary; this
  branch produces `keplorayx-engine` instead, so neither script runs unmodified.

## Details that are still maintained by hand

- `zr/ARCHITECTURE.md` — engine layout, data dirs, RPC. Its frontend section still says GPUI.
- `apps/keplorayx/docs/PLAN-zgx-orbit.md` — Wave 1 phases A–G and their gates.
- `zr/docs/plans/zeron-base.md` and `docs/plans/buddies.md` — Wave 2/3 plans; both lag the code.
