# A textarea can't grow with its text

`ui.textarea` is 156 points tall unless given `h`; `h = "auto"` doesn't size it
to its lines. A composer wants one line that grows to a few, then scrolls.

**Workaround:** the Ray bar's `h` is a function of its text: one line per
newline, plus one per ~72 characters for wrapping, from one to eight lines
(`ui/screens/ray.luau`). An `auto` height measured from the laid-out text,
capped by `max_h`, would replace it exactly.
