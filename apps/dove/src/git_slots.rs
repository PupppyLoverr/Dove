//! A few slots for background work that shells out to git.
//!
//! GPUI's background tasks run on the OS thread pool, which starts another
//! thread whenever the ones it has are blocked, up to 500 on Windows. A git
//! command blocks its thread, so unbounded refreshes grow the process by a
//! thread (and its heap) per command. Waiting here holds no thread, and a
//! task dropped while it waits never runs git at all.

use std::sync::LazyLock;

use flume::{Receiver, Sender};

const SLOTS: usize = 3;

static POOL: LazyLock<(Sender<()>, Receiver<()>)> = LazyLock::new(|| {
    let (tx, rx) = flume::bounded(SLOTS);
    for _ in 0..SLOTS {
        let _ = tx.send(());
    }
    (tx, rx)
});

/// Held while running git; gives the slot back when dropped.
pub struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        let _ = POOL.0.try_send(());
    }
}

/// Wait for a free slot.
pub async fn slot() -> Slot {
    let _ = POOL.1.recv_async().await;
    Slot
}
