# Binary size and idle memory

Budget: the release `dove` binary stays at or under 110 MB (CI's size gate;
plugins and downloadable models excluded), and an idle window renders zero
frames. This page records the October 2026 pass over both.

Measured on Windows 11 x64, `cargo build --release -p dove` (fat LTO,
`codegen-units = 1`, `strip`, `panic = "abort"`), from `feat/live-voice`
(f8f5fcf). Idle numbers come from `scripts/perf_check.ps1`: a scratch
`DOVE_HOME`, `DOVE_SKIP_INTRO=1`, window shown without focus, 30 to 45 s to
settle on Home, then a 10 s sample. The machine was shared with other builds,
so CPU readings carry about ±0.5 % of noise.

## Results

| | Before | After | Change |
|---|---:|---:|---:|
| `dove.exe` | 94,685,696 B (90.3 MB) | 72,713,216 B (69.3 MB) | −21.0 MB (−23 %) |
| Idle RSS, fresh home | 66 MB | 63 MB | −3 MB |
| Idle RSS, Parakeet installed + halftone wallpaper | 760 MB | 90.5 MB | −670 MB |
| Idle CPU (main process, % of one core) | 0.3–1.3 % | 0.6–0.8 % | noise |
| Idle renders in 10 s (`DOVE_FRAME_STATS`) | 0–2 | 0–2 | unchanged |

The second memory row is the setup of a real install: the Parakeet
dictation model and Moonshine call models downloaded, a 2 MB JPEG wallpaper
with the halftone effect. Without the voice models that home idles at
87.5 MB, so the wallpaper costs about 15 MB and Parakeet the rest.

## What changed

### ONNX Runtime is a download, not a link (−21 MB)

`ort` was built with its default features, which link a static ONNX Runtime
1.28 (with DirectML) into `dove.exe`, plus a 18.5 MB `DirectML.dll` copied
next to it. `zeron-voice` and `dove-voice` now build `ort` with
`default-features = false` and `load-dynamic`, and Parakeet's `parakeet-rs`
with `default-features = false, features = ["cpu", "load-dynamic", "api-28"]`.

The library itself comes from Microsoft's official release
(`microsoft/onnxruntime` v1.28.0 on GitHub), pinned in
`vendor/zeron-voice/src/runtime.rs` for Windows x64/arm64, macOS arm64 and
Linux x64/arm64:

- It is fetched only inside a model download the user starts from Settings ›
  Voice (`zeron_voice::download`, `dove_voice::models::download`). Nothing
  downloads at startup and the runtime is never fetched on its own.
- The release archive is checked against its pinned size and SHA-256 as it
  streams in; the library is extracted, checked against its own pinned
  SHA-256, and only then moved into place with a `.verified` sidecar
  (`<DOVE_HOME>/voice/onnxruntime/`).
- Before every load the library is hashed again (as the models are), then
  handed to `ort::init_from`. A missing or changed file is an error in the
  voice UI, never a fallback to some other `onnxruntime.dll` on the system
  (Windows ships one in System32).
- "Installed" for a model now also means the runtime is in place, so an
  install from before this change shows Download again; pressing it fetches
  only the runtime and keeps the model files already verified.
- Removing the last local model removes the runtime too.

Download cost: the Windows archive is 78.8 MB for a 15.8 MB DLL (Microsoft
ships headers, import libraries and PDBs in the same zip). The user already
downloads 30–670 MB of models at that point. See follow-ups for a ranged
fetch that would cut it to about 6 MB.

### Parakeet no longer loads at startup (−670 MB idle)

`VoiceModel::init` used to send Parakeet (≈670 MB once loaded) to the
dictation worker as soon as Keplorayx started, whenever the model was installed.
Dictation already loads the model on the worker while the microphone
records, and the worker drops it after 10 idle minutes anyway, so the
startup warm-up only helped clips shorter than the load time in the first
ten minutes. It is gone; the warm-up right after a fresh install stays.

### File icons embed deflated (−1 MB)

The 357 file-type SVGs (1.4 MB) are deflated by `build.rs` and inflated
when GPUI first asks for an icon (`file_icons.rs`). They were already lazy in
memory (static data, decoded on demand); this only shrinks the binary.

## Looked at, not changed

- **`image` features.** Trimming Keplorayx's list does nothing: gpui (the zui
  git dependency) enables bmp, dds, exr, ff, gif, hdr, ico, jpeg, png, pnm,
  qoi, tga, tiff and webp for every crate, and Cargo merges features. Keplorayx's
  own list matches formats it really decodes (markdown and Office images,
  favicons, wallpapers). The saving would have to happen in zui.
- **Wallpaper.** About 15 MB with a wallpaper set: the effect is rasterized
  once at the hero's device size, kept as a CPU copy beside the GPU atlas
  entry, with up to four cached variants. Reasonable as is.
- **WebView2 and harness prewarm.** Neither runs at idle on Home: the
  browser starts its WebView2 process on first use, and the harness prewarm
  starts while the user types in a composer.
- **Fonts and UI icons.** About 0.8 MB of fonts and 0.2 MB of UI SVGs,
  embedded and paged in on demand.

## Follow-ups

- **Ranged runtime fetch.** Read the zip's central directory with HTTP range
  requests and fetch only `onnxruntime.dll`'s compressed bytes (≈6 MB
  instead of 78.8 MB), checking the extracted library's pinned hash.
- **Download progress for call models.** `dove_voice::models::download`
  reports only the model's bytes, so the runtime part of a first call-model
  download shows no progress. Settings › Voice ignores progress today; when it
  grows a bar, add the runtime's bytes there.
- **Idle renders on a brand-new home.** Both builds render the whole window
  (Root, Shell, Sidebar, Toasts) once every 4–10 s for the first minute on a
  fresh `DOVE_HOME`; a set-up home renders none. Likely first-run work
  (agent detection, skills seeding) notifying the shell. Worth one pass with
  `DOVE_FRAME_STATS` and the views it names.
- **Idle CPU.** 0.3–1 % of one core with zero renders: some background
  poll is still awake. Not traced in this pass.
- **`cargo bloat`.** Not run: it rebuilds without `strip`, which is another
  hour of fat LTO on this machine. The before/after above is the direct
  measure of the voice stack.

## Verification

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace` pass.
- `ZERON_VOICE_RUNTIME=<dir> cargo test -p zeron-voice -- --ignored`
  downloads the pinned Windows x64 release, verifies both hashes and loads it
  through `ort`.
- `DOVE_VOICE_MODELS=<root> cargo test -p dove-voice -- --ignored` runs the
  whole call pipeline (Silero, Smart Turn, Moonshine) on that runtime and
  transcribes each spoken turn correctly.
- Not run end to end: Parakeet dictation in the app on the new runtime (it
  goes through the same `runtime::load`), and the macOS and Linux pins
  (hashes taken from the published archives; the tar reader has a unit test).

## Gates

- CI's size gate stays at 110 MB (`.github/workflows/ci.yml`); the binary
  is now 69.3 MB.
- `scripts/perf_check.ps1 -Exe <dove.exe>` prints exe size, idle RSS and
  private bytes (main and children), idle CPU and the render count for a
  scratch launch. `scripts/perf/perf.ps1` remains the full
  startup/streaming/latency harness.
