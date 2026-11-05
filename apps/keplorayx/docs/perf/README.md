# RAM and CPU: keplorayx on zgx against the GPUI build

Measured on 2026-11-05 on this Windows 11 machine with `scripts/perf/perf.ps1 -Kind kx`. Each run starts 1, 5 or 10 fake-agent sessions on the scheduler's first tick, then measures idle, streaming, and idle again after the sessions. The window is visible but not focused.

The builds compared:
- **keplorayx:** a **debug** build (`C:/dvw/target-kx-zgx/debug/keplorayx.exe`), with Zeron's design system vendored (`ui/ds`) and the notch window open.
- **Dove (GPUI):** its **release** build, `C:/dvw/target-ui-v4/release/dove.exe`, measured on 2026-11-04 against a scratch `DOVE_HOME`, read-only.

## The final build (Zeron's shell, composer, sidebar, transcript, panel)

| Sessions | Own private RAM, streaming | Main CPU, streaming | Frames/s, streaming | Main CPU idle | Frames/s idle | Main CPU after |
|---|---|---|---|---|---|---|
| 1 | 183.9 MB (Dove 139.0) | 5.2 % (Dove 3.1) | 4.0 | 0.83 % | **0** | 0.52 % |
| 5 | 186.0 MB (Dove 157.6) | 5.8 % (Dove 3.8) | 4.2 | 1.67 % | **0** | 1.25 % |
| 10 | 197.8 MB (Dove 171.8) | 16.0 % (Dove 6.1) | 20.7 | 0.62 % | **0** | 0.73 % |

The run just before the shell rework, with Zeron's composer, sidebar and transcript already in, measured streaming CPU at 3.0 / 4.6 / 7.0 % and memory at 148 / 196 / 181 MB.

## What it says

- **Idle draws nothing.** Zero frames at idle, before and after the sessions, in every run.
- **Streaming no longer draws a steady 40 frames a second.** The script-driven breath clock is gone; status rings now run on the GPU and only while seen. At 1 and 5 sessions the window draws about 4 frames a second while agents stream, and CPU is within 1–2 points of Dove's release build.
- **At 10 sessions it's heavier.** About 21 frames a second and 16 % CPU (the previous run measured 7 %). This run varies more than the others, and the debug build is being compared with Dove's release, but it needs a look: likely the sidebar's rows and the transcript's working line all updating as ten threads move at once.
- **Memory went up.** It's now 25–45 MB more than Dove. Before the design system was vendored it was 25–32 % less. Zeron's components, their strings and their assets load into the scripts' heap. Loading only the components a screen uses, or a release build, should bring some of it back. That's not measured yet.

## Not measured

- **Key latency:** zgx has no Windows input trace yet, so it reads n/a.
- **A release build of keplorayx:** not built (debug only, as asked).

## Run it again

```powershell
cargo build -p keplorayx -p dove-escrow
cargo build --release --manifest-path scripts/perf/fake-agent/Cargo.toml
& scripts\perf\perf.ps1 -Kind kx -Exe C:\dvw\target-kx-zgx\debug\keplorayx.exe -FakeAgent C:\dvw\target-kx-zgx\release\dove-fake-agent.exe -Sessions @(1,5,10)
```

Run it from inside PowerShell, as above. Through `powershell -File`, `-Sessions 1,5,10` arrives as the one number 1510.
