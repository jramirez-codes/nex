//! Binary split tree for panes within a tab.

use egui::{Pos2, Rect, Vec2};

pub type PaneId = u64;

/// What a pane in the split tree shows. The app keeps each kind's state in its own map
/// (see `App::pane_kind`); sessions save it per leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Explorer,
    /// A Changes (git diff) pane; in a session, `cwd` is the folder whose repository it shows.
    Diff,
    /// An Agents pane (in a session, `cwd` is unused).
    Agents,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Children side by side (divider is vertical). "Split right".
    Horizontal,
    /// Children stacked (divider is horizontal). "Split down".
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug)]
pub enum Node {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// Fraction of the space given to `a`.
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// A draggable gap between two siblings. `path` addresses the split node from the root
/// (false = `a`, true = `b`).
pub struct Divider {
    pub path: Vec<bool>,
    pub axis: Axis,
    pub rect: Rect,
    /// The rect occupied by the whole split (used to convert pointer position to a ratio).
    pub parent: Rect,
}

/// How deep a walk of the tree goes before [`Panes`] needs the heap: far deeper than the
/// splits anyone fits in a window.
const WALK_DEPTH: usize = 16;

/// The panes of a tree in layout order (see [`Node::iter`]).
pub struct Panes<'a> {
    /// The node to walk next.
    next: Option<&'a Node>,
    /// The `b` sides still owed on the way down, innermost last: the first `owed_len` slots
    /// of `owed`, then `spill` for a tree deeper than [`WALK_DEPTH`] (empty, and so
    /// unallocated, otherwise).
    owed: [Option<&'a Node>; WALK_DEPTH],
    owed_len: usize,
    spill: Vec<&'a Node>,
}

impl<'a> Panes<'a> {
    fn push(&mut self, node: &'a Node) {
        match self.owed.get_mut(self.owed_len) {
            Some(slot) => {
                *slot = Some(node);
                self.owed_len += 1;
            }
            None => self.spill.push(node),
        }
    }

    fn pop(&mut self) -> Option<&'a Node> {
        self.spill.pop().or_else(|| {
            self.owed_len = self.owed_len.checked_sub(1)?;
            self.owed[self.owed_len].take()
        })
    }
}

impl Iterator for Panes<'_> {
    type Item = PaneId;

    fn next(&mut self) -> Option<PaneId> {
        let mut node = self.next.take().or_else(|| self.pop())?;
        loop {
            match node {
                Node::Leaf(id) => return Some(*id),
                Node::Split { a, b, .. } => {
                    self.push(b);
                    node = a;
                }
            }
        }
    }
}

impl Node {
    /// The panes in layout order (`a` before `b`), as a list. To only ask something of them,
    /// use [`iter`](Self::iter), [`contains`](Self::contains) or [`len`](Self::len), which
    /// don't allocate.
    pub fn panes(&self) -> Vec<PaneId> {
        self.iter().collect()
    }

    /// The panes in layout order (`a` before `b`), without building a list.
    pub fn iter(&self) -> Panes<'_> {
        Panes { next: Some(self), owed: [None; WALK_DEPTH], owed_len: 0, spill: Vec::new() }
    }

    /// Whether pane `id` is in this tree.
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(leaf) => *leaf == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// How many panes the tree holds (never zero).
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        match self {
            Node::Leaf(_) => 1,
            Node::Split { a, b, .. } => a.len() + b.len(),
        }
    }

    /// Replace leaf `target` with a split containing it and `new` (new pane goes right/below).
    pub fn split(&mut self, target: PaneId, axis: Axis, new: PaneId) -> bool {
        let dir = match axis {
            Axis::Horizontal => Direction::Right,
            Axis::Vertical => Direction::Down,
        };
        self.insert(target, dir, new)
    }

    /// Replace leaf `target` with an even split holding it and `new`, `new` on the `dir` side.
    pub fn insert(&mut self, target: PaneId, dir: Direction, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let axis = match dir {
                    Direction::Left | Direction::Right => Axis::Horizontal,
                    Direction::Up | Direction::Down => Axis::Vertical,
                };
                let (t, n) = (Box::new(Node::Leaf(target)), Box::new(Node::Leaf(new)));
                let (a, b) = match dir {
                    Direction::Left | Direction::Up => (n, t),
                    Direction::Right | Direction::Down => (t, n),
                };
                *self = Node::Split { axis, ratio: 0.5, a, b };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.insert(target, dir, new) || b.insert(target, dir, new),
        }
    }

    /// Move leaf `pane` to the `dir` side of leaf `target` (its old slot closes up). Returns
    /// false (changing nothing) if either is missing or they are the same pane.
    pub fn move_pane(&mut self, pane: PaneId, target: PaneId, dir: Direction) -> bool {
        if pane == target || !self.contains(pane) || !self.contains(target) {
            return false;
        }
        self.remove(pane) && self.insert(target, dir, pane)
    }

    /// Remove a leaf; its sibling takes the parent's place. Returns false if `target` is the
    /// root leaf (caller should close the whole tab) or isn't present.
    pub fn remove(&mut self, target: PaneId) -> bool {
        let Node::Split { a, b, .. } = self else {
            return false;
        };
        let survivor = if matches!(**a, Node::Leaf(id) if id == target) {
            Some(std::mem::replace(&mut **b, Node::Leaf(0)))
        } else if matches!(**b, Node::Leaf(id) if id == target) {
            Some(std::mem::replace(&mut **a, Node::Leaf(0)))
        } else {
            None
        };
        match survivor {
            Some(node) => {
                *self = node;
                true
            }
            None => a.remove(target) || b.remove(target),
        }
    }

    /// Exchange the leaves holding `a` and `b`, keeping every split's axis and ratio.
    /// Returns false (changing nothing) if either is missing or they are the same pane.
    pub fn swap(&mut self, a: PaneId, b: PaneId) -> bool {
        if a == b || !self.contains(a) || !self.contains(b) {
            return false;
        }
        self.swap_leaves(a, b);
        true
    }

    fn swap_leaves(&mut self, a: PaneId, b: PaneId) {
        match self {
            Node::Leaf(id) if *id == a => *id = b,
            Node::Leaf(id) if *id == b => *id = a,
            Node::Leaf(_) => {}
            Node::Split { a: na, b: nb, .. } => {
                na.swap_leaves(a, b);
                nb.swap_leaves(a, b);
            }
        }
    }

    /// Put `pane` along the whole `dir` edge of this tree, taking `frac` of the space.
    pub fn wrap_edge(&mut self, pane: PaneId, dir: Direction, frac: f32) {
        let frac = frac.clamp(0.1, 0.9);
        let axis = match dir {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        };
        let rest = Box::new(std::mem::replace(self, Node::Leaf(0)));
        let leaf = Box::new(Node::Leaf(pane));
        *self = match dir {
            Direction::Left | Direction::Up => Node::Split { axis, ratio: frac, a: leaf, b: rest },
            Direction::Right | Direction::Down => Node::Split { axis, ratio: 1.0 - frac, a: rest, b: leaf },
        };
    }

    pub fn ratio_mut(&mut self, path: &[bool]) -> Option<&mut f32> {
        match self {
            Node::Leaf(_) => None,
            Node::Split { ratio, a, b, .. } => match path.split_first() {
                None => Some(ratio),
                Some((false, rest)) => a.ratio_mut(rest),
                Some((true, rest)) => b.ratio_mut(rest),
            },
        }
    }

    pub fn equalize(&mut self) {
        if let Node::Split { ratio, a, b, axis } = self {
            // Weight by the number of panes along the same axis so three panes split evenly.
            let (na, nb) = (a.count_along(*axis), b.count_along(*axis));
            *ratio = na as f32 / (na + nb) as f32;
            a.equalize();
            b.equalize();
        }
    }

    fn count_along(&self, along: Axis) -> usize {
        match self {
            Node::Leaf(_) => 1,
            Node::Split { axis, a, b, .. } if *axis == along => a.count_along(along) + b.count_along(along),
            Node::Split { a, b, .. } => a.count_along(along).max(b.count_along(along)),
        }
    }

    /// Path to the innermost split of `axis` that contains `target`, plus whether target is
    /// in that split's `b` side.
    fn enclosing_split(&self, target: PaneId, axis: Axis) -> Option<(Vec<bool>, bool)> {
        fn walk(n: &Node, target: PaneId, axis: Axis, path: &mut Vec<bool>, best: &mut Option<(Vec<bool>, bool)>) -> bool {
            match n {
                Node::Leaf(id) => *id == target,
                Node::Split { axis: ax, a, b, .. } => {
                    for (side, child) in [(false, a), (true, b)] {
                        path.push(side);
                        let found = walk(child, target, axis, path, best);
                        path.pop();
                        if found {
                            if *ax == axis && best.is_none() {
                                *best = Some((path.clone(), side));
                            }
                            return true;
                        }
                    }
                    false
                }
            }
        }
        let mut best = None;
        walk(self, target, axis, &mut Vec::new(), &mut best);
        best
    }

    /// Grow the pane `target` toward `dir` by `amount` (fraction of the split).
    pub fn resize(&mut self, target: PaneId, dir: Direction, amount: f32) {
        let axis = match dir {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        };
        if let Some((path, _)) = self.enclosing_split(target, axis)
            && let Some(r) = self.ratio_mut(&path)
        {
            let delta = match dir {
                Direction::Right | Direction::Down => amount,
                Direction::Left | Direction::Up => -amount,
            };
            *r = (*r + delta).clamp(0.1, 0.9);
        }
    }

    /// Lay out panes in `rect`, leaving `gap` between siblings.
    pub fn layout(&self, rect: Rect, gap: f32, panes: &mut Vec<(PaneId, Rect)>, dividers: &mut Vec<Divider>) {
        self.layout_inner(rect, gap, &mut Vec::new(), panes, dividers);
    }

    fn layout_inner(&self, rect: Rect, gap: f32, path: &mut Vec<bool>, panes: &mut Vec<(PaneId, Rect)>, dividers: &mut Vec<Divider>) {
        match self {
            Node::Leaf(id) => panes.push((*id, rect)),
            Node::Split { axis, ratio, a, b } => {
                let (ra, div, rb) = match axis {
                    Axis::Horizontal => {
                        let x = (rect.left() + (rect.width() - gap) * ratio).round();
                        (
                            Rect::from_min_max(rect.min, Pos2::new(x, rect.bottom())),
                            Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + gap, rect.bottom())),
                            Rect::from_min_max(Pos2::new(x + gap, rect.top()), rect.max),
                        )
                    }
                    Axis::Vertical => {
                        let y = (rect.top() + (rect.height() - gap) * ratio).round();
                        (
                            Rect::from_min_max(rect.min, Pos2::new(rect.right(), y)),
                            Rect::from_min_max(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y + gap)),
                            Rect::from_min_max(Pos2::new(rect.left(), y + gap), rect.max),
                        )
                    }
                };
                dividers.push(Divider { path: path.clone(), axis: *axis, rect: div, parent: rect });
                path.push(false);
                a.layout_inner(ra, gap, path, panes, dividers);
                path.pop();
                path.push(true);
                b.layout_inner(rb, gap, path, panes, dividers);
                path.pop();
            }
        }
    }
}

/// Which edge of `area` the pane at `rect` sits along, and how much of the area it takes
/// across that edge: a pane spanning the full height on the right is `(Right, width share)`.
/// A pane touching no full edge goes on the side it is nearest. `(Right, 0.4)` if `area`
/// is empty.
pub fn edge_placement(rect: Rect, area: Rect) -> (Direction, f32) {
    if !area.is_positive() || !rect.is_positive() {
        return (Direction::Right, 0.4);
    }
    const SLACK: f32 = 2.0;
    let full_h = rect.top() <= area.top() + SLACK && rect.bottom() >= area.bottom() - SLACK;
    let full_w = rect.left() <= area.left() + SLACK && rect.right() >= area.right() - SLACK;
    let wfrac = (rect.width() / area.width()).clamp(0.1, 0.9);
    let hfrac = (rect.height() / area.height()).clamp(0.1, 0.9);
    let (dl, dr) = (rect.left() - area.left(), area.right() - rect.right());
    let (du, dd) = (rect.top() - area.top(), area.bottom() - rect.bottom());
    let side = |left: bool| if left { Direction::Left } else { Direction::Right };
    let vside = |up: bool| if up { Direction::Up } else { Direction::Down };
    if full_h && !full_w {
        return (side(dl < dr), wfrac);
    }
    if full_w && !full_h {
        return (vside(du < dd), hfrac);
    }
    if full_w && full_h {
        return (Direction::Right, 0.4);
    }
    // Nearest edge, preferring left/right on ties (columns read best beside other panes).
    let (dh, dv) = (dl.min(dr), du.min(dd));
    if dh <= dv { (side(dl < dr), wfrac) } else { (vside(du < dd), hfrac) }
}

/// Pick the pane whose rect lies in `dir` from `from`, preferring the nearest one that
/// overlaps on the perpendicular axis.
pub fn neighbor(rects: &[(PaneId, Rect)], from: PaneId, dir: Direction) -> Option<PaneId> {
    let (_, src) = *rects.iter().find(|(id, _)| *id == from)?;
    let c = src.center();
    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter(|(_, r)| match dir {
            Direction::Left => r.right() <= src.left() + 1.0,
            Direction::Right => r.left() >= src.right() - 1.0,
            Direction::Up => r.bottom() <= src.top() + 1.0,
            Direction::Down => r.top() >= src.bottom() - 1.0,
        })
        .min_by(|(_, a), (_, b)| {
            let score = |r: &Rect| {
                let d: Vec2 = r.center() - c;
                let (along, across, overlaps) = match dir {
                    Direction::Left | Direction::Right => (d.x.abs(), d.y.abs(), r.top() < src.bottom() && r.bottom() > src.top()),
                    _ => (d.y.abs(), d.x.abs(), r.left() < src.right() && r.right() > src.left()),
                };
                along + across * if overlaps { 0.25 } else { 4.0 }
            };
            score(a).total_cmp(&score(b))
        })
        .map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(axis: Axis, ratio: f32, a: Node, b: Node) -> Node {
        Node::Split { axis, ratio, a: Box::new(a), b: Box::new(b) }
    }

    /// A chain of `depth` splits, each leaf numbered in layout order from 1. With `left`,
    /// the chain runs down the `a` sides (the deepest walk); else down the `b` sides.
    fn chain(depth: usize, left: bool) -> Node {
        let mut n = Node::Leaf(if left { 1 } else { depth as PaneId + 1 });
        for k in 1..=depth as PaneId {
            n = if left {
                split(Axis::Horizontal, 0.5, n, Node::Leaf(k + 1))
            } else {
                split(Axis::Vertical, 0.5, Node::Leaf(depth as PaneId + 1 - k), n)
            };
        }
        n
    }

    #[test]
    fn iter_contains_and_len_agree_with_panes() {
        let leaf = Node::Leaf(7);
        assert_eq!(leaf.iter().collect::<Vec<_>>(), vec![7]);
        assert_eq!((leaf.len(), leaf.contains(7), leaf.contains(1)), (1, true, false));

        let n = split(
            Axis::Horizontal,
            0.5,
            split(Axis::Vertical, 0.5, Node::Leaf(4), Node::Leaf(2)),
            split(Axis::Vertical, 0.5, Node::Leaf(9), split(Axis::Horizontal, 0.5, Node::Leaf(1), Node::Leaf(3))),
        );
        assert_eq!(n.iter().collect::<Vec<_>>(), vec![4, 2, 9, 1, 3]);
        assert_eq!(n.panes(), vec![4, 2, 9, 1, 3]);
        assert_eq!(n.len(), 5);
        assert!(n.panes().iter().all(|&id| n.contains(id)));
        assert!(!n.contains(5));
    }

    /// The walk keeps its place in a fixed array, and still gets a tree deeper than that right.
    #[test]
    fn iter_walks_trees_of_any_depth_in_order() {
        for depth in [1, WALK_DEPTH - 1, WALK_DEPTH, WALK_DEPTH + 1, 3 * WALK_DEPTH] {
            for left in [true, false] {
                let n = chain(depth, left);
                let want: Vec<PaneId> = (1..=depth as PaneId + 1).collect();
                assert_eq!(n.iter().collect::<Vec<_>>(), want, "depth {depth}, left {left}");
                assert_eq!(n.len(), depth + 1);
            }
        }
        // Within the array's depth nothing is put on the heap.
        let n = chain(WALK_DEPTH, true);
        let mut it = n.iter();
        while it.next().is_some() {
            assert_eq!(it.spill.capacity(), 0);
        }
    }

    #[test]
    fn swap_exchanges_leaves_and_keeps_the_split() {
        let mut n = split(Axis::Horizontal, 0.3, Node::Leaf(1), Node::Leaf(2));
        assert!(n.swap(1, 2));
        assert_eq!(n.panes(), vec![2, 1]);
        let Node::Split { axis, ratio, .. } = n else { panic!() };
        assert_eq!((axis, ratio), (Axis::Horizontal, 0.3));
    }

    #[test]
    fn swap_across_nested_splits() {
        let mut n = split(Axis::Horizontal, 0.5, Node::Leaf(1), split(Axis::Vertical, 0.5, Node::Leaf(2), Node::Leaf(3)));
        assert!(n.swap(3, 1));
        assert_eq!(n.panes(), vec![3, 2, 1]);
    }

    #[test]
    fn swap_with_missing_or_same_pane_does_nothing() {
        let mut n = split(Axis::Vertical, 0.5, Node::Leaf(1), Node::Leaf(2));
        assert!(!n.swap(1, 9));
        assert!(!n.swap(1, 1));
        assert_eq!(n.panes(), vec![1, 2]);
    }

    #[test]
    fn move_side_by_side_to_stacked() {
        let mut n = split(Axis::Horizontal, 0.5, Node::Leaf(1), Node::Leaf(2));
        assert!(n.move_pane(1, 2, Direction::Down));
        assert_eq!(n.panes(), vec![2, 1]);
        let Node::Split { axis, .. } = n else { panic!() };
        assert_eq!(axis, Axis::Vertical);
    }

    #[test]
    fn move_into_a_nested_split() {
        let mut n = split(Axis::Horizontal, 0.5, Node::Leaf(1), split(Axis::Vertical, 0.5, Node::Leaf(2), Node::Leaf(3)));
        assert!(n.move_pane(1, 3, Direction::Left));
        // 1 left the root, so the vertical split is the root now, with 3 split into [1 | 3].
        let Node::Split { axis, b, .. } = &n else { panic!() };
        assert_eq!(*axis, Axis::Vertical);
        assert!(matches!(**b, Node::Split { axis: Axis::Horizontal, .. }));
        assert_eq!(n.panes(), vec![2, 1, 3]);
    }

    #[test]
    fn wrap_edge_puts_the_pane_on_that_side() {
        let mut n = split(Axis::Vertical, 0.5, Node::Leaf(1), Node::Leaf(2));
        n.wrap_edge(9, Direction::Right, 0.3);
        let Node::Split { axis, ratio, b, .. } = &n else { panic!() };
        assert_eq!(*axis, Axis::Horizontal);
        assert!((ratio - 0.7).abs() < 1e-6);
        assert!(matches!(**b, Node::Leaf(9)));
        let mut n = Node::Leaf(1);
        n.wrap_edge(9, Direction::Up, 0.25);
        assert_eq!(n.panes(), vec![9, 1]);
        let Node::Split { axis, ratio, .. } = n else { panic!() };
        assert_eq!((axis, ratio), (Axis::Vertical, 0.25));
    }

    #[test]
    fn edge_placement_finds_the_side_and_share() {
        let area = Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 50.0));
        let right = Rect::from_min_max(Pos2::new(60.0, 0.0), Pos2::new(100.0, 50.0));
        assert_eq!(edge_placement(right, area), (Direction::Right, 0.4));
        let bottom = Rect::from_min_max(Pos2::new(0.0, 25.0), Pos2::new(100.0, 50.0));
        assert_eq!(edge_placement(bottom, area), (Direction::Down, 0.5));
        let corner = Rect::from_min_max(Pos2::ZERO, Pos2::new(30.0, 40.0));
        assert_eq!(edge_placement(corner, area), (Direction::Left, 0.3));
        assert_eq!(edge_placement(area, area), (Direction::Right, 0.4));
    }

    #[test]
    fn move_onto_itself_or_missing_does_nothing() {
        let mut n = split(Axis::Horizontal, 0.5, Node::Leaf(1), Node::Leaf(2));
        assert!(!n.move_pane(1, 1, Direction::Up));
        assert!(!n.move_pane(1, 9, Direction::Up));
        assert!(!n.move_pane(9, 1, Direction::Up));
        assert_eq!(n.panes(), vec![1, 2]);
    }
}
