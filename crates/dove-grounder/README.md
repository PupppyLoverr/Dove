# dove-grounder

The Local Grounder: a Rust port of nexi-cua v0.4.0's decision policy (Apache-2.0, see the repository's
`THIRD_PARTY_NOTICES.md`). Given a window's snapshot rows and an explicit intent ("click Save"), it picks one
action with no LLM call, or abstains and returns its best candidate rows. Dove's `computer` tool
(`crates/dove-computer`) uses it for `act` and for `{"intent": ...}` steps in `run`, over the rows cua-driver's
`get_window_state` returns.

## The network

`assets/nexi-cua-1.onnx` is NEXI-CUA-1 (fp32, 6.4 MB), built into Dove with the default `embedded-model` feature
(`service::embedded`). It was exported from nexi-cua's `weights/nexi-cua-1.pt` (SHA-256 `6ffb91b0…9cbc`, checked
against `RELEASE_CHECKSUMS.sha256`):

```sh
python -m venv venv && venv/bin/pip install torch onnx numpy
git archive origin/main nexi-cua | tar -x -C /tmp      # in PupppyLoverr/NEXI-Models-Drivers
venv/bin/python scripts/export_onnx.py /tmp/nexi-cua assets/nexi-cua-1.onnx
```

The network loads on the first decision and unloads after 300 s without one. Without it (Settings › Plugins ›
Computer use › Local grounder off), `act::ground_with(None, …)` decides with the deterministic resolver alone:
full accessible names, ordinals, key chords; it abstains whenever the resolver defers. `--fp16` writes a 3.2 MB
file that does not pass the gate (it ranks 3 abstained near-ties differently).

A file can stand in for the built-in asset: `DOVE_DRIVER_GROUNDER_MODEL=<path>` (`Grounder::from_env`).

## The gate

```sh
export DOVE_DRIVER_GROUNDER_MODEL=$PWD/assets/nexi-cua-1.onnx
cargo test --release -p dove-grounder                       # gate.rs, memory.rs, synth_port.rs
```

- `tests/gate.rs`: identical decisions and top-ranked candidates to the Python policy on nexi-cua's 5 captured pages
  and 2,000 seeded synthetic screens, with and without the resolver, and nexi-cua's own scores on them. Without
  weights it still checks every decision the resolver makes.
- `tests/memory.rs`: the grounder adds under 30 MB RSS (release build) and unloads when idle.
- `tests/synth_port.rs`: with `DOVE_GROUNDER_FULL_GOLDEN` set to goldens written with `--with-screens`, checks the
  Rust port of nexi-cua's generator field by field.

`tests/goldens/nexi-cua-0.4.0.json.gz` is written by `scripts/make_goldens.py` (run it with nexi-cua on
`PYTHONPATH` and CPU torch).
