//! A tab's layout: a binary tree of splits whose leaves are panes. Pure data,
//! so splitting, closing, resizing and focus moves are tested without a window.

use serde::{Deserialize, Serialize};

pub type PaneId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    /// Side by side: a vertical seam, split right.
    Row,
    /// Stacked: a horizontal seam, split down.
    Column,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// The first child's share of the space, 0..1.
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// How far a seam may go: no pane narrower than this share.
pub const MIN_RATIO: f32 = 0.12;

impl Node {
    /// Leaves left to right, top to bottom.
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

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(l) => *l == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// Put `new` beside `target`, sharing its space evenly. False when
    /// `target` is not in the tree.
    pub fn split(&mut self, target: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split { axis, ratio: 0.5, a: Box::new(Node::Leaf(target)), b: Box::new(Node::Leaf(new)) };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, axis, new) || b.split(target, axis, new),
        }
    }

    /// Take `target` out; its sibling takes the parent's place. `None` when
    /// it was the last pane (the tab closes).
    pub fn remove(self, target: PaneId) -> Option<Node> {
        match self {
            Node::Leaf(id) if id == target => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split { axis, ratio, a, b } => match (a.remove(target), b.remove(target)) {
                (Some(a), Some(b)) => Some(Node::Split { axis, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    /// Set the ratio of the split at `path` (0 = first child, 1 = second,
    /// from the root), clamped so neither side vanishes.
    pub fn set_ratio(&mut self, path: &[u8], ratio: f32) {
        let mut node = self;
        for step in path {
            match node {
                Node::Split { a, b, .. } => node = if *step == 0 { a } else { b },
                Node::Leaf(_) => return,
            }
        }
        if let Node::Split { ratio: r, .. } = node {
            *r = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        }
    }

    /// An even grid of `ids` in reading order: as square as it gets, rows
    /// stacked, the last row taking whatever is left.
    pub fn grid(ids: &[PaneId]) -> Option<Node> {
        if ids.is_empty() {
            return None;
        }
        let cols = (ids.len() as f32).sqrt().ceil() as usize;
        let rows: Vec<Node> =
            ids.chunks(cols).map(|row| chain(row.iter().map(|id| Node::Leaf(*id)).collect(), Axis::Row)).collect();
        Some(chain(rows, Axis::Column))
    }

    /// The next pane after `id` in reading order, wrapping.
    pub fn next(&self, id: PaneId, forward: bool) -> Option<PaneId> {
        let leaves = self.leaves();
        let ix = leaves.iter().position(|l| *l == id)?;
        let n = leaves.len();
        Some(leaves[if forward { (ix + 1) % n } else { (ix + n - 1) % n }])
    }
}

/// `nodes` one after another along `axis`, sharing the space evenly.
fn chain(mut nodes: Vec<Node>, axis: Axis) -> Node {
    if nodes.len() == 1 {
        return nodes.remove(0);
    }
    let k = nodes.len() as f32;
    let first = nodes.remove(0);
    Node::Split { axis, ratio: 1.0 / k, a: Box::new(first), b: Box::new(chain(nodes, axis)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_are_square_and_even() {
        assert_eq!(Node::grid(&[]), None);
        assert_eq!(Node::grid(&[1]), Some(Node::Leaf(1)));
        let four = Node::grid(&[1, 2, 3, 4]).unwrap();
        assert_eq!(four.leaves(), vec![1, 2, 3, 4]);
        // Two rows of two.
        match &four {
            Node::Split { axis: Axis::Column, ratio, a, .. } => {
                assert!((ratio - 0.5).abs() < 1e-6);
                assert!(matches!(**a, Node::Split { axis: Axis::Row, .. }));
            }
            other => panic!("unexpected {other:?}"),
        }
        // Nine: three rows, the first a third of the height.
        let nine: Vec<PaneId> = (1..=9).collect();
        match Node::grid(&nine).unwrap() {
            Node::Split { axis: Axis::Column, ratio, .. } => assert!((ratio - 1.0 / 3.0).abs() < 1e-6),
            other => panic!("unexpected {other:?}"),
        }
        let sixteen: Vec<PaneId> = (1..=16).collect();
        assert_eq!(Node::grid(&sixteen).unwrap().leaves(), sixteen);
    }

    #[test]
    fn split_right_then_down_matches_the_recording_layout() {
        // One agent on the left, two shells stacked on the right.
        let mut t = Node::Leaf(1);
        assert!(t.split(1, Axis::Row, 2));
        assert!(t.split(2, Axis::Column, 3));
        assert_eq!(t.leaves(), vec![1, 2, 3]);
        match &t {
            Node::Split { axis: Axis::Row, a, b, .. } => {
                assert_eq!(**a, Node::Leaf(1));
                assert!(matches!(**b, Node::Split { axis: Axis::Column, .. }));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!t.split(9, Axis::Row, 4));
    }

    #[test]
    fn removing_collapses_to_the_sibling() {
        let mut t = Node::Leaf(1);
        t.split(1, Axis::Row, 2);
        t.split(2, Axis::Column, 3);
        let t = t.remove(2).unwrap();
        assert_eq!(t.leaves(), vec![1, 3]);
        let t = t.remove(1).unwrap();
        assert_eq!(t, Node::Leaf(3));
        assert_eq!(t.remove(3), None);
    }

    #[test]
    fn ratios_clamp_and_focus_wraps() {
        let mut t = Node::Leaf(1);
        t.split(1, Axis::Row, 2);
        t.split(2, Axis::Column, 3);
        t.set_ratio(&[], 0.0);
        t.set_ratio(&[1], 0.7);
        match &t {
            Node::Split { ratio, b, .. } => {
                assert_eq!(*ratio, MIN_RATIO);
                assert!(matches!(**b, Node::Split { ratio, .. } if (ratio - 0.7).abs() < 1e-6));
            }
            _ => unreachable!(),
        }
        assert_eq!(t.next(3, true), Some(1));
        assert_eq!(t.next(1, false), Some(3));
        assert_eq!(t.next(7, true), None);
    }

    #[test]
    fn layout_round_trips_through_json() {
        let mut t = Node::Leaf(1);
        t.split(1, Axis::Column, 2);
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Node>(&s).unwrap(), t);
    }
}
