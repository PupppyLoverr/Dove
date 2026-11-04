# No colour per border side

`border_sides` sets each side's width but there is one `border_color`. Orbit's
"lip of light" is a lifted surface's top edge brighter than its other edges.

**Workaround:** a 1-point inset shadow along the top (`inset_shadow =
"$kx.lip.inset"`, `0 1 0 0 {kx.lip}`) over a faint hairline all round
(`ui/kx/surface.luau`). It reads the same, but can't be dashed or animated as a
border can.
