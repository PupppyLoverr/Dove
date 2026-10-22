//! Free splits for the side panel, like Claude's: a tree of panes, each
//! showing one surface (a browser, a terminal, a side chat, the diff…).
//! The chat column counts as one pane, so the dock holds up to seven of the
//! eight. This module is the layout alone (pure, so it is tested here); the
//! side panel owns what each pane shows and draws it.

/// A pane's id, stable while the pane lives.
pub type PaneId = u32;

/// Panes in the window, the chat column included.
pub const MAX_PANES: usize = 8;
/// The smallest a pane may be split down to.
pub const MIN_W: f32 = 320.0;
pub const MIN_H: f32 = 220.0;

/// How a split lays out its two halves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    /// Side by side ("split right").
    Row,
    /// Stacked ("split down").
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    fn halves(&self, axis: Axis, ratio: f32) -> (Rect, Rect) {
        match axis {
            Axis::Row => {
                let w = self.w * ratio;
                (Rect::new(self.x, self.y, w, self.h), Rect::new(self.x + w, self.y, self.w - w, self.h))
            }
            Axis::Column => {
                let h = self.h * ratio;
                (Rect::new(self.x, self.y, self.w, h), Rect::new(self.x, self.y + h, self.w, self.h - h))
            }
        }
    }
}

/// The layout: a pane, or a split of two layouts.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Node {
    Leaf(PaneId),
    Split { axis: Axis, ratio: f32, a: Box<Node>, b: Box<Node> },
}

impl Default for Node {
    fn default() -> Self {
        Node::Leaf(0)
    }
}

/// Where a split sits in the tree, for dragging its divider: one bit per
/// level (0 = first half, 1 = second), under a leading 1.
pub type SplitPath = u32;

pub const ROOT: SplitPath = 1;

fn child(path: SplitPath, second: bool) -> SplitPath {
    (path << 1) | second as u32
}

impl Node {
    /// Every pane, in reading order.
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Node::Leaf(_) => 1,
            Node::Split { a, b, .. } => a.count() + b.count(),
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(x) => *x == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// Split pane `at` in two along `axis`; `new` takes the second half.
    pub fn split(&mut self, at: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == at => {
                *self = Node::Split { axis, ratio: 0.5, a: Box::new(Node::Leaf(at)), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(at, axis, new) || b.split(at, axis, new),
        }
    }

    /// Close pane `id`: its sibling takes the split's place. The last pane
    /// stays.
    pub fn remove(&mut self, id: PaneId) -> bool {
        let Node::Split { a, b, .. } = self else { return false };
        if **a == Node::Leaf(id) {
            *self = (**b).clone();
            return true;
        }
        if **b == Node::Leaf(id) {
            *self = (**a).clone();
            return true;
        }
        a.remove(id) || b.remove(id)
    }

    /// The split at `path`, if there is one.
    fn at_path(&mut self, path: SplitPath) -> Option<&mut Node> {
        if path <= ROOT {
            return Some(self);
        }
        let depth = 31 - path.leading_zeros();
        let mut node = self;
        for level in (0..depth).rev() {
            let second = (path >> level) & 1 == 1;
            node = match node {
                Node::Split { a, b, .. } => {
                    if second {
                        b
                    } else {
                        a
                    }
                }
                Node::Leaf(_) => return None,
            };
        }
        Some(node)
    }

    /// Move the divider of the split at `path`.
    pub fn set_ratio(&mut self, path: SplitPath, ratio: f32) {
        if let Some(Node::Split { ratio: r, .. }) = self.at_path(path) {
            *r = ratio.clamp(0.1, 0.9);
        }
    }

    /// Each pane's rectangle inside `bounds`.
    pub fn rects(&self, bounds: Rect) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.place(bounds, &mut out);
        out
    }

    fn place(&self, r: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, r)),
            Node::Split { axis, ratio, a, b } => {
                let (ra, rb) = r.halves(*axis, *ratio);
                a.place(ra, out);
                b.place(rb, out);
            }
        }
    }

    /// The layout with only the panes `keep` accepts (None when none are).
    pub fn prune(&self, keep: &impl Fn(PaneId) -> bool) -> Option<Node> {
        match self {
            Node::Leaf(id) => keep(*id).then_some(Node::Leaf(*id)),
            Node::Split { axis, ratio, a, b } => match (a.prune(keep), b.prune(keep)) {
                (Some(a), Some(b)) => Some(Node::Split { axis: *axis, ratio: *ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(x), None) | (None, Some(x)) => Some(x),
                (None, None) => None,
            },
        }
    }

    /// Every split with its path, axis and ratio.
    #[cfg(test)]
    pub fn splits(&self) -> Vec<(SplitPath, Axis, f32)> {
        let mut out = Vec::new();
        self.walk(ROOT, &mut out);
        out
    }

    #[cfg(test)]
    fn walk(&self, path: SplitPath, out: &mut Vec<(SplitPath, Axis, f32)>) {
        if let Node::Split { axis, ratio, a, b } = self {
            out.push((path, *axis, *ratio));
            a.walk(child(path, false), out);
            b.walk(child(path, true), out);
        }
    }

    /// The path of a child split, for drawing nested dividers.
    pub fn child_path(path: SplitPath, second: bool) -> SplitPath {
        child(path, second)
    }
}

/// Whether pane `at` can split along `axis` inside `bounds`: there is
/// room for two panes of the minimum size, and fewer than `max` panes.
pub fn can_split(layout: &Node, at: PaneId, axis: Axis, bounds: Rect, max: usize) -> bool {
    if layout.count() >= max {
        return false;
    }
    let Some((_, r)) = layout.rects(bounds).into_iter().find(|(id, _)| *id == at) else { return false };
    match axis {
        Axis::Row => r.w / 2.0 >= MIN_W,
        Axis::Column => r.h / 2.0 >= MIN_H,
    }
}

/// The panes that fit in `bounds`: while one is squeezed under half the
/// minimum size, the least recently used one is folded away (it comes
/// back when there is room). `recent` lists panes, most recent first.
pub fn fit(layout: &Node, bounds: Rect, recent: &[PaneId]) -> Node {
    let mut shown = layout.clone();
    while shown.count() > 1 {
        let squeezed = shown.rects(bounds).iter().any(|(_, r)| r.w < MIN_W / 2.0 || r.h < MIN_H / 2.0);
        if !squeezed {
            break;
        }
        let leaves = shown.leaves();
        let rank = |id: PaneId| recent.iter().position(|x| *x == id).unwrap_or(usize::MAX);
        let Some(victim) = leaves.iter().copied().max_by_key(|id| rank(*id)) else { break };
        shown = shown.prune(&|id| id != victim).unwrap_or(shown);
    }
    shown
}

/// The pane next to `from` in direction `dir`, by pane centres.
pub fn neighbor(layout: &Node, from: PaneId, dir: Dir, bounds: Rect) -> Option<PaneId> {
    let rects = layout.rects(bounds);
    let (_, here) = rects.iter().find(|(id, _)| *id == from)?;
    let (cx, cy) = here.center();
    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, r)| {
            let (x, y) = r.center();
            let (dx, dy) = (x - cx, y - cy);
            let ahead = match dir {
                Dir::Left => r.x + r.w <= here.x + 1.0 && dx < 0.0,
                Dir::Right => r.x >= here.x + here.w - 1.0 && dx > 0.0,
                Dir::Up => r.y + r.h <= here.y + 1.0 && dy < 0.0,
                Dir::Down => r.y >= here.y + here.h - 1.0 && dy > 0.0,
            };
            ahead.then_some((*id, dx * dx + dy * dy))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// The ratio for a divider dragged to `pos` along a split of `extent`
/// pixels starting at `start`, keeping both halves at least `min` where
/// the split is big enough.
pub fn drag_ratio(pos: f32, start: f32, extent: f32, min: f32) -> f32 {
    if extent <= 0.0 {
        return 0.5;
    }
    let edge = (min / extent).clamp(0.1, 0.5);
    ((pos - start) / extent).clamp(edge, 1.0 - edge)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn four() -> Node {
        // 0 | 1 on top of 2 | 3.
        let mut n = Node::Leaf(0);
        assert!(n.split(0, Axis::Column, 2));
        assert!(n.split(0, Axis::Row, 1));
        assert!(n.split(2, Axis::Row, 3));
        n
    }

    #[test]
    fn splits_close_and_place_panes() {
        let mut n = four();
        assert_eq!(n.leaves(), vec![0, 1, 2, 3]);
        let rects = n.rects(Rect::new(0.0, 0.0, 1000.0, 800.0));
        assert_eq!(rects[1], (1, Rect::new(500.0, 0.0, 500.0, 400.0)));
        assert_eq!(rects[2], (2, Rect::new(0.0, 400.0, 500.0, 400.0)));
        assert!(n.remove(1));
        assert_eq!(n.leaves(), vec![0, 2, 3]);
        assert_eq!(n.rects(Rect::new(0.0, 0.0, 1000.0, 800.0))[0].1.w, 1000.0);
        assert!(!Node::Leaf(0).remove(0), "the last pane stays");
        assert!(!n.split(9, Axis::Row, 10));
    }

    #[test]
    fn dividers_move_by_path() {
        let mut n = four();
        let paths: Vec<SplitPath> = n.splits().iter().map(|s| s.0).collect();
        assert_eq!(paths, vec![ROOT, 0b10, 0b11]);
        n.set_ratio(0b11, 0.25);
        let r = n.rects(Rect::new(0.0, 0.0, 1000.0, 800.0));
        assert_eq!(r[2].1.w, 250.0);
        n.set_ratio(ROOT, 0.0);
        assert_eq!(n.splits()[0].2, 0.1, "clamped");
        assert_eq!(drag_ratio(50.0, 0.0, 1000.0, MIN_W), 0.32);
        assert_eq!(drag_ratio(600.0, 100.0, 1000.0, MIN_W), 0.5);
    }

    #[test]
    fn splits_need_room_and_stop_at_the_cap() {
        let n = Node::Leaf(0);
        let wide = Rect::new(0.0, 0.0, 700.0, 300.0);
        assert!(can_split(&n, 0, Axis::Row, wide, 7));
        assert!(!can_split(&n, 0, Axis::Column, wide, 7), "150px halves are too short");
        assert!(!can_split(&four(), 0, Axis::Row, Rect::new(0.0, 0.0, 4000.0, 4000.0), 4));
    }

    #[test]
    fn squeezed_panes_fold_least_recent_first() {
        let n = four();
        let roomy = Rect::new(0.0, 0.0, 1400.0, 900.0);
        assert_eq!(fit(&n, roomy, &[0]).count(), 4);
        // 300px wide: each column is 150, under half of MIN_W.
        let narrow = Rect::new(0.0, 0.0, 300.0, 900.0);
        let shown = fit(&n, narrow, &[3, 0, 2, 1]);
        assert_eq!(shown.leaves(), vec![0, 3], "1 and 2 folded, the recent ones stay");
        let tiny = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(fit(&n, tiny, &[2]).leaves(), vec![2], "the focused pane always shows");
    }

    #[test]
    fn layouts_round_trip_through_json() {
        let n = four();
        let back: Node = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(back, n);
    }

    #[test]
    fn focus_moves_to_the_neighbouring_pane() {
        let n = four();
        let b = Rect::new(0.0, 0.0, 1000.0, 800.0);
        assert_eq!(neighbor(&n, 0, Dir::Right, b), Some(1));
        assert_eq!(neighbor(&n, 0, Dir::Down, b), Some(2));
        assert_eq!(neighbor(&n, 3, Dir::Up, b), Some(1));
        assert_eq!(neighbor(&n, 0, Dir::Left, b), None);
    }
}
