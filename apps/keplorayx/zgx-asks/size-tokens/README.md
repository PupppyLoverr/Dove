# A size given as a token has no effect

`styling.md` says lengths and sizes take tokens. `max_w = "$kx.measure"`
(a number token declared with `ui.theme.tokens`) left the column unbounded,
while `max_w = 680` bounds it:

```luau
ui.theme.tokens { app = { measure = 680 } }
return ui.column { max_w = "$app.measure", ui.text { string.rep("word ", 400), wrap = true } }
```

**Workaround:** `kx.MEASURE = 680`, the number the token holds
(`ui/kx/init.luau`), used wherever a reading measure is set.
