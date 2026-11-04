# RAM and CPU: keplorayx on zgx against the GPUI build

Measured on 2026-11-04, on this Windows 11 machine, with `scripts/perf/perf.ps1`. Each run starts 1, 5 or 10 fake-agent sessions on the scheduler's first tick and measures idle, then streaming, then idle again after the sessions.

The window is visible but not focused, as the harness does by default. The full tables are `keplorayx-debug.md` and `dove-gpui-release.md`, with the raw data in the `.json` files beside them.

- **keplorayx:** a **debug** build (dev profile, optimised with debuginfo), `C:/dvw/target-kx-zgx/debug/keplorayx.exe`, 70 MB. It includes the notch window.
- **Dove (GPUI):** the **release** build in `C:/dvw/target-ui-v4/release/dove.exe`, 105 MB. It was run read-only against a scratch `DOVE_HOME`.

| Sessions | | Own private RAM, streaming | Main CPU, streaming (% of a core) | Main CPU idle | Main CPU after | Frames/s idle | Window shown |
|---|---|---|---|---|---|---|---|
| 1 | keplorayx | **105.5 MB** | 7.0 | 0.31 | 0.42 | **0** | 803 ms |
| 1 | Dove | 139.0 MB (+0.9 % other) | 3.1 | 0.52 | 0.52 | n/a | 667 ms |
| 5 | keplorayx | **110.0 MB** | 8.0 | 0.21 | 0.52 | **0** | 828 ms |
| 5 | Dove | 157.6 MB (+1.1 % other) | 3.8 | 0.00 | 0.31 | n/a | 583 ms |
| 10 | keplorayx | **116.5 MB** | 9.3 | 0.31 | 0.31 | **0** | 736 ms |
| 10 | Dove | 171.8 MB (+1.1 % other) | 6.1 | 0.21 | 0.21 | n/a | 725 ms |

## What it says

- **Memory: keplorayx uses 25 to 32 % less**, and it grows more slowly with sessions: +11 MB from 1 to 10 sessions, against Dove's +33 MB.
- **Idle:**
  - keplorayx draws **no frames at all** when nothing moves, before and after the sessions (`ZGX_PRESENT_LOG`). That meets the "idle frames = 0" gate.
  - Idle CPU is a fraction of a percent for both.
- **Streaming CPU is higher.** keplorayx's main process uses about 2 to 4 points more than Dove's, and it renders a steady 40 frames a second while anything streams.
  - The 40 is flat whatever the session count, which points at a frame cap or one animation, not per-token work.
  - Two caveats on the comparison:
    1. keplorayx here is a debug build against Dove's release build.
    2. Dove's work is split: about 1 % of it sits in a helper process ("other" above).

  Next steps: a release build of keplorayx, then finding what holds 40 frames a second while streaming. Candidates are the breath clock (30 fps when a live indicator is on screen) and the transcript's `follow`.
- **Window shown:** keplorayx's window takes about 0.1 to 0.2 s longer to appear. Most of that is the debug build loading its scripts before the first frame.

## Not measured

- **Key latency:** zgx has no Windows input trace yet (`zgx-asks/input-trace-windows`), so it reads n/a for keplorayx.
- **Dove's frame rate:** n/a, because its release build doesn't emit `DOVE_FRAME_STATS`.
- **Binary size gate (115 MB):** the release keplorayx binary wasn't built tonight (debug only, as asked). The debug binary is 70 MB.

## Run it again

```powershell
cargo build -p keplorayx -p dove-escrow
cargo build --release --manifest-path scripts/perf/fake-agent/Cargo.toml
& scripts\perf\perf.ps1 -Exe C:\dvw\target-kx-zgx\debug\keplorayx.exe -FakeAgent C:\dvw\target-kx-zgx\release\dove-fake-agent.exe -Sessions @(1,5,10)
```

Run it from inside PowerShell, as above. Through `powershell -File`, `-Sessions 1,5,10` arrives as the one number 1510, so pass `@(1,5,10)` from a PowerShell prompt.
