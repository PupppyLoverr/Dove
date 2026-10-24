//! Keeping Dove's memory bounded over long sessions (revamp §3.6: P7, P13,
//! D7 and the 1 GB ceiling).
//!
//! - **Tab hibernation (P7).** Closing a thread tab starts a 30 s grace.
//!   When it ends and no agent or terminal is live in the thread, its whole
//!   `ThreadView` (transcript rows, composer, list state) is dropped.
//!   Reopening the tab rebuilds it from disk; the composer's draft lives in
//!   `draft.<task>`. A thread that is still busy is retried when it goes
//!   idle (its status or view mode changes), never by polling.
//! - **Web-view hibernation (D7).** Ten minutes after a thread tab closes,
//!   its hidden browser tabs and canvases drop their native web views; each
//!   reloads its URL when shown again.
//! - **LRU caps (P13).** [`Lru`] bounds the Mermaid and canvas caches;
//!   [`ImageLru`] bounds decoded `img()` resources to about 128 MB, dropping
//!   the ones that went off screen first.
//! - **The 1 GB watchdog.** Nothing polls: the process's memory is read on
//!   events that already happen (a tab closes, the side panel hides, the
//!   window deactivates). Above the ceiling, hidden web views hibernate,
//!   closed tabs hibernate at once and the caches are trimmed.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dove_core::proto::{TaskId, TaskStatus};
use gpui::{
    App, AppContext as _, Context, Entity, EntityId, Global, ImageCache, ImageCacheError, RenderImage, Resource,
    RetainAllImageCache, Subscription, Window,
};

use crate::shell::{Route, Shell};
use crate::thread::{TaskViewEventKind, ThreadEvent};

/// How long a closed thread tab keeps its view.
pub const THREAD_GRACE: Duration = Duration::from_secs(30);
/// How long a closed thread tab keeps its hidden web views.
pub const WEB_GRACE: Duration = Duration::from_secs(10 * 60);
/// The memory ceiling for Dove's main process.
pub const CEILING: u64 = 1024 * 1024 * 1024;
/// Decoded images kept by [`ImageLru`].
pub const IMAGE_BUDGET: usize = 128 * 1024 * 1024;
/// An image drawn this recently counts as on screen and is never evicted.
const ON_SCREEN: Duration = Duration::from_secs(2);

// ------------------------------------------------------------------ LRU

/// A least-recently-used map bounded by entry count and by a total cost
/// (bytes, usually) that the caller gives per entry.
pub struct Lru<K, V> {
    map: HashMap<K, Slot<V>>,
    tick: u64,
    cost: usize,
    max_entries: usize,
    max_cost: usize,
}

struct Slot<V> {
    value: V,
    used: u64,
    cost: usize,
}

impl<K: Eq + Hash + Clone, V> Lru<K, V> {
    pub fn new(max_entries: usize, max_cost: usize) -> Self {
        Self { map: HashMap::new(), tick: 0, cost: 0, max_entries: max_entries.max(1), max_cost }
    }

    /// The value for `key`, marking it most recently used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|s| {
            s.used = tick;
            &s.value
        })
    }

    #[cfg(test)]
    pub fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// Insert (or replace) `key`, then evict the least recently used
    /// entries past the bounds. Returns what was evicted (never `key`).
    pub fn insert(&mut self, key: K, value: V, cost: usize) -> Vec<(K, V)> {
        self.tick += 1;
        if let Some(old) = self.map.insert(key.clone(), Slot { value, used: self.tick, cost }) {
            self.cost -= old.cost;
        }
        self.cost += cost;
        self.shrink_to(self.max_entries, self.max_cost, Some(&key))
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let slot = self.map.remove(key)?;
        self.cost -= slot.cost;
        Some(slot.value)
    }

    /// Evict least recently used entries until at most `entries` remain
    /// costing at most `cost`, sparing `keep`.
    pub fn shrink_to(&mut self, entries: usize, cost: usize, keep: Option<&K>) -> Vec<(K, V)> {
        let mut out = Vec::new();
        while self.map.len() > entries || self.cost > cost {
            let victim =
                self.map.iter().filter(|(k, _)| Some(*k) != keep).min_by_key(|(_, s)| s.used).map(|(k, _)| k.clone());
            let Some(k) = victim else { break };
            if let Some(v) = self.remove(&k) {
                out.push((k, v));
            }
        }
        out
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The summed cost of the entries.
    #[cfg(test)]
    pub fn cost(&self) -> usize {
        self.cost
    }
}

// ------------------------------------------------------------ images

/// GPUI's retain-all image cache behind an LRU byte budget: images not
/// drawn for a while (off screen) are dropped first, and only once the
/// budget is exceeded or the watchdog trims.
pub struct ImageLru {
    inner: Entity<RetainAllImageCache>,
    items: HashMap<u64, Seen>,
    bytes: usize,
    budget: usize,
    /// The watchdog asked for a trim; done on the next load (it has the window).
    trim_pending: bool,
}

struct Seen {
    resource: Resource,
    used: Instant,
    bytes: usize,
}

impl ImageLru {
    fn new(budget: usize, cx: &mut App) -> Self {
        Self { inner: RetainAllImageCache::new(cx), items: HashMap::new(), bytes: 0, budget, trim_pending: false }
    }

    /// Drop images not drawn in the last moment, oldest first, until the
    /// cache holds at most `max` bytes. `all_offscreen` drops every image
    /// that is not on screen whatever the size.
    fn evict(&mut self, max: usize, all_offscreen: bool, window: &mut Window, cx: &mut App) {
        if !all_offscreen && self.bytes <= max {
            return;
        }
        let now = Instant::now();
        let mut order: Vec<(u64, Instant)> =
            self.items.iter().filter(|(_, s)| now - s.used >= ON_SCREEN).map(|(k, s)| (*k, s.used)).collect();
        order.sort_by_key(|(_, used)| *used);
        for (key, _) in order {
            if !all_offscreen && self.bytes <= max {
                break;
            }
            if let Some(seen) = self.items.remove(&key) {
                self.bytes -= seen.bytes;
                self.inner.update(cx, |c, cx| c.remove(&seen.resource, window, cx));
            }
        }
    }
}

fn image_bytes(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .map(|f| {
            let size = image.size(f);
            (size.width.0.max(0) as usize) * (size.height.0.max(0) as usize) * 4
        })
        .sum()
}

impl ImageCache for ImageLru {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        if std::mem::take(&mut self.trim_pending) {
            self.evict(self.budget / 4, true, window, cx);
        }
        let key = gpui::hash(resource);
        let result = self.inner.update(cx, |c, cx| c.load(resource, window, cx));
        let seen = self.items.entry(key).or_insert_with(|| Seen {
            resource: resource.clone(),
            used: Instant::now(),
            bytes: 0,
        });
        seen.used = Instant::now();
        if seen.bytes == 0
            && let Some(Ok(image)) = &result
        {
            seen.bytes = image_bytes(image).max(1);
            self.bytes += seen.bytes;
            if self.bytes > self.budget {
                self.evict(self.budget, false, window, cx);
            }
        }
        result
    }
}

// ------------------------------------------------------------ state

#[derive(Default)]
struct Memory {
    /// Closed thread tabs waiting out their grace.
    threads: HashMap<TaskId, gpui::Task<()>>,
    webs: HashMap<TaskId, gpui::Task<()>>,
    /// The shell's subscription to each thread view, dropped with it.
    subs: HashMap<TaskId, Subscription>,
    /// The (chat view, change stamp) last sent to the side panel.
    panel_key: Option<(EntityId, u64)>,
    images: Option<Entity<ImageLru>>,
}

impl Global for Memory {}

fn state(cx: &mut App) -> &mut Memory {
    if !cx.has_global::<Memory>() {
        cx.set_global(Memory::default());
    }
    cx.global_mut::<Memory>()
}

/// The shared, bounded image cache for `img()` resources.
pub fn image_cache(cx: &mut App) -> Entity<ImageLru> {
    if let Some(e) = state(cx).images.clone() {
        return e;
    }
    let e = cx.new(|cx| ImageLru::new(IMAGE_BUDGET, cx));
    state(cx).images = Some(e.clone());
    e
}

/// Keep the shell's subscription to thread `id`'s view; it is dropped when
/// the view hibernates (or replaced when the view is rebuilt).
pub fn own_thread_sub(id: TaskId, sub: Subscription, cx: &mut App) {
    state(cx).subs.insert(id, sub);
}

/// Whether the side panel needs the chat's todos, subagents and designs
/// again: `key` is the chat view and its change stamp.
pub fn panel_changed(key: (EntityId, u64), cx: &mut App) -> bool {
    let m = state(cx);
    if m.panel_key == Some(key) {
        return false;
    }
    m.panel_key = Some(key);
    true
}

// ------------------------------------------------------------ hibernation

/// A thread tab closed: start the graces, and check the ceiling.
pub fn tab_closed(shell: &mut Shell, id: TaskId, window: &mut Window, cx: &mut Context<Shell>) {
    arm_thread(id, cx);
    let web = cx.spawn(async move |this, cx| {
        cx.background_executor().timer(WEB_GRACE).await;
        let _ = this.update(cx, |shell, cx| {
            let busy = shell.task(id).is_some_and(|t| t.status == TaskStatus::Running);
            if !shell.tabs.contains(&id) && shell.route != Route::Thread(id) && !busy {
                shell.panel.update(cx, |p, cx| p.hibernate_thread(id, cx));
            }
        });
    });
    state(cx).webs.insert(id, web);
    watchdog(shell, Some(window), cx);
}

/// A thread tab opened (or reopened): its views stay.
pub fn tab_opened(id: TaskId, cx: &mut App) {
    let m = state(cx);
    m.threads.remove(&id);
    m.webs.remove(&id);
}

fn arm_thread(id: TaskId, cx: &mut Context<Shell>) {
    let task = cx.spawn(async move |this, cx| {
        cx.background_executor().timer(THREAD_GRACE).await;
        let _ = this.update(cx, |shell, cx| hibernate_thread(shell, id, cx));
    });
    state(cx).threads.insert(id, task);
}

/// A thread reported a status or view-mode change. If its tab is closed and
/// it just went idle, give it a fresh grace (it was busy when the last one
/// ended).
pub fn thread_event(shell: &Shell, id: TaskId, ev: &ThreadEvent, cx: &mut Context<Shell>) {
    let idle = match ev {
        ThreadEvent::Mode => true,
        ThreadEvent::Chat(TaskViewEventKind::Status(s)) => *s != TaskStatus::Running,
        ThreadEvent::Chat(_) => false,
    };
    if idle && !shell.keeps_thread(id, cx) && shell.threads.contains_key(&id) {
        arm_thread(id, cx);
    }
}

/// Drop thread `id`'s view if its tab is still closed and nothing in it is
/// live. Its web views are left to their own grace.
fn hibernate_thread(shell: &mut Shell, id: TaskId, cx: &mut Context<Shell>) -> bool {
    if shell.keeps_thread(id, cx) {
        return false;
    }
    let Some(view) = shell.threads.get(&id) else { return false };
    if !view.read(cx).can_hibernate(cx) {
        return false;
    }
    shell.threads.remove(&id);
    state(cx).subs.remove(&id);
    tracing::debug!("hibernated thread {id}");
    true
}

// ------------------------------------------------------------ watchdog

/// Read the process's memory and, above [`CEILING`], hibernate hidden web
/// views and closed tabs and trim the caches. Called on events that happen
/// anyway; does nothing below the ceiling.
pub fn watchdog(shell: &mut Shell, window: Option<&mut Window>, cx: &mut Context<Shell>) {
    let Some(used) = process_memory() else { return };
    if used < CEILING {
        return;
    }
    tracing::warn!("memory {} MB is over the {} MB ceiling; trimming", used >> 20, CEILING >> 20);
    shell.panel.update(cx, |p, cx| p.hibernate_hidden(cx));
    let closed: Vec<TaskId> = shell.threads.keys().filter(|id| !shell.keeps_thread(**id, cx)).copied().collect();
    for id in closed {
        hibernate_thread(shell, id, cx);
    }
    trim_caches(window, cx);
}

/// Shrink every cache to its floor.
pub fn trim_caches(window: Option<&mut Window>, cx: &mut App) {
    crate::markdown_extra::trim(window, cx);
    // Images are dropped on their next load, which has the window.
    let images = image_cache(cx);
    images.update(cx, |c, _| c.trim_pending = true);
}

/// Dove's own memory use in bytes: private bytes on Windows, the physical
/// footprint on macOS, the resident set on Linux.
pub fn process_memory() -> Option<u64> {
    imp::process_memory()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub fn process_memory() -> Option<u64> {
        let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        // SAFETY: the pseudo handle needs no closing; `counters` is a valid,
        // correctly sized out-parameter.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), counters.cb) };
        (ok != 0).then_some(counters.PrivateUsage as u64)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    /// `rusage_info_v2`'s leading fields, up to the footprint.
    #[repr(C)]
    struct RusageInfo {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        pkg_idle_wkups: u64,
        interrupt_wkups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        phys_footprint: u64,
        rest: [u64; 32],
    }

    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfo) -> i32;
    }

    pub fn process_memory() -> Option<u64> {
        const RUSAGE_INFO_V2: i32 = 2;
        let mut info: RusageInfo = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is larger than `rusage_info_v2` and zeroed.
        let rc = unsafe { proc_pid_rusage(std::process::id() as i32, RUSAGE_INFO_V2, &mut info) };
        (rc == 0).then_some(info.phys_footprint)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn process_memory() -> Option<u64> {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let resident: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        // SAFETY: sysconf has no preconditions.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        Some(resident * u64::try_from(page).ok().filter(|p| *p > 0).unwrap_or(4096))
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod imp {
    pub fn process_memory() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lru_evicts_the_least_recently_used() {
        let mut lru: Lru<&str, u32> = Lru::new(2, usize::MAX);
        assert!(lru.insert("a", 1, 0).is_empty());
        assert!(lru.insert("b", 2, 0).is_empty());
        // Touching `a` makes `b` the oldest.
        assert_eq!(lru.get(&"a"), Some(&1));
        assert_eq!(lru.insert("c", 3, 0), vec![("b", 2)]);
        assert!(lru.contains(&"a") && lru.contains(&"c"));
        assert_eq!(lru.len(), 2);
    }

    #[test]
    fn lru_respects_a_cost_budget() {
        let mut lru: Lru<u32, ()> = Lru::new(100, 10);
        lru.insert(1, (), 4);
        lru.insert(2, (), 4);
        assert_eq!(lru.cost(), 8);
        // 4 + 4 + 6 > 10: the oldest goes, never the new entry.
        assert_eq!(lru.insert(3, (), 6).into_iter().map(|(k, _)| k).collect::<Vec<_>>(), vec![1]);
        assert_eq!(lru.cost(), 10);
        // A single entry over budget is kept (it is the one just asked for).
        assert_eq!(lru.insert(4, (), 50).len(), 2);
        assert_eq!((lru.len(), lru.cost()), (1, 50));
        // Replacing an entry replaces its cost.
        lru.insert(4, (), 1);
        assert_eq!(lru.cost(), 1);
        assert_eq!(lru.shrink_to(0, 0, None).len(), 1);
        assert!(lru.is_empty());
    }

    #[test]
    fn process_memory_is_measured() {
        if cfg!(any(windows, target_os = "macos", target_os = "linux")) {
            let used = process_memory().expect("a reading");
            assert!(used > 1024 * 1024, "{used} bytes is implausibly small");
            assert!(used < CEILING * 64);
        }
    }
}
