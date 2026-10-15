I read the module and its call sites before changing anything. The render path builds every row on each frame, so the cost grows with the length of the history.

## What I found

1. The list renders all rows with `.children(...)`, which means layout work is proportional to the whole transcript.
2. Each streamed delta calls `cx.notify()`, and the parent view re-renders with it.
3. Decoded images stay in memory after the thread closes, because nothing evicts them.

| Area | Before | After |
|---|---|---|
| Rows laid out | every row | visible rows only |
| Notifies per batch | one per delta | one per batch |
| Image memory | grows | bounded by an LRU |

Here is the change to the row builder:

```rust
fn render_rows(&self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
    range
        .filter_map(|ix| self.rows.get(ix))
        .map(|row| row.render(&self.theme, cx))
        .collect()
}
```

And the batch drain now coalesces deltas before it touches the view:

```rust
while let Ok(first) = rx.recv_async().await {
    let mut batch = vec![first];
    batch.extend(rx.try_iter());
    this.update(cx, |view, cx| view.apply_batch(batch, cx))?;
}
```

> I kept the public API the same, so callers do not change.

### Verification

- `cargo check -p dove` passes.
- The transcript with 5,000 rows scrolls at a steady frame rate.
- Memory after closing ten threads returns close to the starting point.

Next I will run the benchmark again with ten concurrent sessions and compare the numbers with the baseline. If the idle render count is still above zero, I will trace which entity notifies and remove the timer that causes it.

