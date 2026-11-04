# No key-to-frame trace on Windows

Dove's perf harness measures keypress latency from `DOVE_INPUT_TRACE` (key
to frame start) plus GPUI's frame durations. zgx's `ZGX_TRACE_KEYS` traces
macOS's ⌘ keys only, and `ZGX_PRESENT_LOG` stamps presents, not inputs.

**Workaround:** `scripts/perf/perf.ps1 -Kind kx` reports frames from
`ZGX_PRESENT_LOG`'s DXGI present count and key latency as n/a. A trace line
when a key event arrives (performance-counter stamped, as the present log is)
would let the harness pair each key with the present after it.
