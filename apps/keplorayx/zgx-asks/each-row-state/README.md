# `ui.each`: the row's state argument arrives as nil

The docs say a `build` that takes a third parameter gets its item as state and
updates in place. With a keyed `ui.each` over a computed list, the third
argument was nil on Windows at `f01b47a0`:

```luau
local items = ui.computed(function() return { { id = "a", title = "One" } } end)
return ui.each(items, function(_item, _i, row)
	return ui.text { row:get().title } -- error: attempt to index nil with 'get'
end, { key = function(t) return t.id end })
```

**Workaround:** rows build from their item (`function(item) … end`). A changed
item rebuilds only its own keyed row, so the others keep their nodes; the cost
is one row's rebuild per change rather than an in-place update.
(`ui/screens/sidebar.luau`, `home.luau`, `inbox.luau`, `ray.luau`)
