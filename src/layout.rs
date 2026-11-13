//! Binary split tree for panes within a tab.

use egui::{Pos2, Rect, Vec2};

pub type PaneId = u64;

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

#[derive(Debug)]
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

impl Node {
    pub fn panes(&self) -> Vec<PaneId> {
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

    /// Replace leaf `target` with a split containing it and `new` (new pane goes right/below).
    pub fn split(&mut self, target: PaneId, axis: Axis, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split {
                    axis,
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(target)),
                    b: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, axis, new) || b.split(target, axis, new),
        }
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
            Node::Split { axis, a, b, .. } if *axis == along => {
                a.count_along(along) + b.count_along(along)
            }
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
        if let Some((path, _)) = self.enclosing_split(target, axis) {
            if let Some(r) = self.ratio_mut(&path) {
                let delta = match dir {
                    Direction::Right | Direction::Down => amount,
                    Direction::Left | Direction::Up => -amount,
                };
                *r = (*r + delta).clamp(0.1, 0.9);
            }
        }
    }

    /// Lay out panes in `rect`, leaving `gap` between siblings.
    pub fn layout(&self, rect: Rect, gap: f32, panes: &mut Vec<(PaneId, Rect)>, dividers: &mut Vec<Divider>) {
        self.layout_inner(rect, gap, &mut Vec::new(), panes, dividers);
    }

    fn layout_inner(
        &self,
        rect: Rect,
        gap: f32,
        path: &mut Vec<bool>,
        panes: &mut Vec<(PaneId, Rect)>,
        dividers: &mut Vec<Divider>,
    ) {
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
                    Direction::Left | Direction::Right => {
                        (d.x.abs(), d.y.abs(), r.top() < src.bottom() && r.bottom() > src.top())
                    }
                    _ => (d.y.abs(), d.x.abs(), r.left() < src.right() && r.right() > src.left()),
                };
                along + across * if overlaps { 0.25 } else { 4.0 }
            };
            score(a).total_cmp(&score(b))
        })
        .map(|(id, _)| *id)
}
