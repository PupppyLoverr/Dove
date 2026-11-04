# What zgx couldn't do (yet), and what Keplorayx does instead

One folder per gap found while building Keplorayx on zgx at `f01b47a0`. Each says
what happened, the smallest repro, and the workaround in use, so it can go
upstream as an issue and the workaround can come out when it's fixed.

| Gap | Workaround here |
|---|---|
| [each-row-state](each-row-state/) | `ui.each`'s third `row` argument arrived as nil, so rows are built from their item (a changed item rebuilds only its own keyed row). |
| [size-tokens](size-tokens/) | `max_w = "$kx.measure"` had no effect; sizes use the number (`kx.MEASURE`). |
| [textarea-auto-height](textarea-auto-height/) | A textarea is 156 points tall unless told; the Ray bar computes its height from its lines. |
| [border-side-colour](border-side-colour/) | No per-side border colour; the lip of light is a 1-point inset shadow (`$kx.lip.inset`). |
| [popup-glass-windows](popup-glass-windows/) | Popups have no system backdrop on Windows; menus and popovers draw solid there (`material.popups_see_through`). |
| [input-trace-windows](input-trace-windows/) | No key-to-present trace on Windows (ZGX_TRACE_KEYS is macOS's); scripts/perf reports key latency as n/a for keplorayx. |
