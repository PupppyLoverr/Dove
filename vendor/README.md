# Vendored crates

## gpui-base

`zeronsh/gpui-component` at `701ca97af7983bf85343d2d1d404ab528b18ec96`,
`crates/base` (Apache-2.0, Longbridge). Wired in through `[patch]` in the
workspace `Cargo.toml`.

Dove change, `src/input/base/blink_cursor.rs`: the caret blinks for about ten
seconds after the last input, then stays solid until the next edit or caret
move. Upstream blinks forever, and every blink renders a full frame
(including backdrop blur), which made an idle window with a focused input
cost about 2–6% of a CPU core.

To update: copy `crates/base` from the new revision, keep this manifest's
dependency list in step with upstream, and re-apply the `MAX_BLINKS` change.

## zeron-proto, zeron-harness, zeron-voice-media

From `zeronsh/zeron` (MIT, Copyright (c) 2026 Wing), `crates/proto`, `crates/harness`
and `crates/voice-media` at workspace version 0.2.112. They supply the agent drivers
Dove shares with Zeron (Claude, Codex, Cursor, OpenCode, Pi, Devin, Grok, Hermes,
Antigravity), wired in through `crates/dove-harness/src/external.rs`.

Dove changes:
- Manifests rewritten with concrete dependency versions (Zeron's use workspace inheritance).
- `zeron-voice-media` no longer depends on `zeron-voice-session`: the
  `VoiceMediaEndpoint` impl for `DesktopMedia` (Codex cloud voice) and its test are
  removed. This drops `zeron-rpc`, `zeron-sync`, `zeron-doc` and `loro` from Dove's build.
- `tests/`, `benches/` and `examples/` are not copied.
- `harness/src/opencode/mod.rs`: `basic_auth_user` picks the `serve` Basic-auth
  user from the executable name, so the same driver runs Kilo Code (an opencode
  fork whose server user is `kilo`). Re-apply after updating.

To update: rerun the vendoring script (see git history of this file) against a newer Zeron checkout.

## zeron-voice

`crates/voice` from Zeron (MIT): local dictation with NVIDIA Parakeet TDT 0.6B v3
(INT8 ONNX, CC-BY-4.0) through `parakeet-rs`, microphone capture through `cpal`,
and the pinned model manifest (`model.json`, SHA-256 per file). Dove changes only
the manifest (concrete dependency versions). The model is downloaded on demand
from Settings › Voice into `~/.dove/voice/parakeet-v3`, never bundled.

Dove additions to the other vendored crates:

- `zeron-proto`: `AgentEvent::RateLimits` and `RateLimitWindow`.
- `zeron-harness`: Claude reads `rate_limit_event` utilization/reset times and
  routes `can_use_tool` approvals through the host's input bridge (Dove asks the
  user unless the permission level answers), honours `dove.permissionMode` and
  `dove.addDirs` model options; Codex emits `account/rateLimits/updated`; ACP
  tool permissions go through the bridge too.
