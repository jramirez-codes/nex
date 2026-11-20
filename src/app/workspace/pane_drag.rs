//! Dragging a pane by its header. Dropped on the middle of another pane of its tab it swaps
//! with it; on one of its edges it moves to that side. Dropped on the title bar (or sidebar) it gets a
//! tab of its own; held over another tab, that tab opens so the pane can be placed in it, and
//! let go on the tab it goes beside that tab's focused pane.

use std::collections::HashMap;

use egui::{Event, PointerButton, Pos2, Rect, Vec2};

use crate::layout::{Direction, PaneId};

use crate::app::actions::Action;
use crate::app::{App, PANE_GAP};

/// How long a dragged pane is held over a tab before that tab opens.
pub(in crate::app) const SPRING_DELAY: f64 = 0.5;
/// How far into a pane, as a fraction of its size, its edges take a drop.
const EDGE_BAND: f32 = 0.25;
/// The most room a held pane takes while over the title bar, so the tabs stay visible.
const MINI: Vec2 = Vec2::new(200.0, 124.0);

/// What letting go of a dragged pane would do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) enum DropTarget {
    /// Trade places with this pane (of the dragged pane's own tab).
    Swap(PaneId),
    /// Move to this side of this pane.
    Edge(PaneId, Direction),
    /// Move into this tab (by uid).
    Tab(u64),
    /// Move into a new tab of its own.
    NewTab,
}

/// A pane being dragged by its header.
pub(in crate::app) struct PaneDrag {
    pub(in crate::app) pane: PaneId,
    /// The uid of the tab it is in.
    pub(in crate::app) home: u64,
    /// Where the pointer holds the card, from the card's top-left corner.
    pub(in crate::app) grab: Vec2,
    /// The last known pointer position.
    pub(in crate::app) at: Pos2,
    /// Its rect in its own tab's layout: its size while held, and what its terminal stays
    /// sized for while another tab is showing.
    pub(in crate::app) slot: Rect,
    /// What a drop would do right now.
    pub(in crate::app) over: Option<DropTarget>,
    /// The tab under the pointer and when it got there; it opens after `SPRING_DELAY`.
    pub(in crate::app) hover: Option<(u64, f64)>,
    /// Whether it may leave its tab (panes of the Files tab stay in it).
    pub(in crate::app) roams: bool,
}

/// What a header drag means for this frame.
#[derive(Default)]
pub(in crate::app) struct DragFrame {
    /// Where cards head instead of their layout rect (the drop preview).
    pub(in crate::app) goals: HashMap<PaneId, Rect>,
    /// The dragged pane and where it is held.
    pub(in crate::app) held: Option<(PaneId, Rect)>,
    /// The held card is shrunk (over the title bar): its content scales with it.
    pub(in crate::app) mini: bool,
    /// What to do, if the pane was just dropped.
    pub(in crate::app) dropped: Option<Action>,
}

/// Which edge band of `r` holds `p`, or `None` for its middle. The nearest edge wins where
/// two bands overlap (the corners).
pub(in crate::app) fn zone(r: Rect, p: Pos2) -> Option<Direction> {
    let (fx, fy) = ((p.x - r.left()) / r.width(), (p.y - r.top()) / r.height());
    [(fx, Direction::Left), (1.0 - fx, Direction::Right), (fy, Direction::Up), (1.0 - fy, Direction::Down)]
        .into_iter()
        .filter(|&(d, _)| d < EDGE_BAND)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, dir)| dir)
}

impl App {
    /// Pick up `pane` (in the shown tab, laid out at `slot`) by its header, held at `at`.
    pub(in crate::app) fn start_pane_drag(&mut self, pane: PaneId, at: Pos2, slot: Rect) {
        let Some(home) = self.cur_tab().map(|t| t.uid) else { return };
        let roams = !self.explorer_active;
        self.pane_drag = Some(PaneDrag { pane, home, grab: at - slot.min, at, slot, over: None, hover: None, roams });
    }

    /// Follow a pane being dragged by its header: track the pointer, preview the drop, open
    /// a tab it is held over, and finish when the button is let go. `rects` is the shown
    /// tab's layout in `area`.
    pub(in crate::app) fn track_pane_drag(&mut self, ctx: &egui::Context, area: Rect, rects: &[(PaneId, Rect)]) -> DragFrame {
        let none = DragFrame::default();
        let Some((pane, home, roams)) = self.pane_drag.as_ref().map(|d| (d.pane, d.home, d.roams)) else { return none };
        let Some((shown, zoomed)) = self.cur_tab().map(|t| (t.uid, t.zoomed)) else { return none };
        let at_home = shown == home;
        let owner = self.tabs.iter().chain(self.explorer_tab.iter()).find(|t| t.contains(pane)).map(|t| t.uid);
        let slot = rects.iter().find(|(id, _)| *id == pane).map(|&(_, r)| r);
        // Nothing to drag it onto in a tab of one pane, unless it can go to another tab.
        let stuck = rects.len() < 2 && !(roams && self.tabs.len() > 1);
        if owner != Some(home) || (at_home && (zoomed || slot.is_none() || stuck)) {
            // The pane closed or moved, or its tab was zoomed.
            self.pane_drag = None;
            self.kick_shown(ctx);
            return none;
        }

        // Where the button came up, if it did this frame: after a touch lifts, some compositors
        // move the pointer back to the parked mouse cursor in the same frame.
        let (released, latest, down, now) = ctx.input(|i| {
            let released = i.events.iter().rev().find_map(|e| match e {
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, .. } => Some(*pos),
                _ => None,
            });
            (released, i.pointer.latest_pos(), i.pointer.primary_down(), i.time)
        });
        let at = {
            let d = self.pane_drag.as_mut().expect("drag checked above");
            if let Some(p) = released.or(latest) {
                d.at = p;
            }
            if let Some(s) = slot {
                d.slot = s;
            }
            d.at
        };

        // Hit-test the layout's slots, not the moving cards, so the preview can't flicker.
        // The middle of a pane swaps with it in the pane's own tab; brought from another
        // tab, the pane moves in beside it (right of a wide pane, below a tall one).
        let over = if area.contains(at) {
            rects.iter().find(|(id, r)| *id != pane && r.contains(at)).map(|&(id, r)| match zone(r, at) {
                Some(dir) => DropTarget::Edge(id, dir),
                None if at_home => DropTarget::Swap(id),
                None if r.height() > r.width() => DropTarget::Edge(id, Direction::Down),
                None => DropTarget::Edge(id, Direction::Right),
            })
        } else if roams {
            self.strip_target(at, shown, home)
        } else {
            None
        };

        // Held over another tab: open it once the delay is up.
        let mut spring = None;
        {
            let d = self.pane_drag.as_mut().expect("drag checked above");
            match over {
                Some(DropTarget::Tab(uid)) if uid != shown => {
                    let since = match d.hover {
                        Some((u, t)) if u == uid => t,
                        _ => {
                            d.hover = Some((uid, now));
                            now
                        }
                    };
                    if now - since >= SPRING_DELAY {
                        spring = Some(uid);
                        d.hover = None;
                    } else {
                        ctx.request_repaint();
                    }
                }
                _ => d.hover = None,
            }
        }
        let over = if spring.is_some() { None } else { over };
        let changed = self.pane_drag.as_mut().is_some_and(|d| std::mem::replace(&mut d.over, over) != over);
        if changed {
            self.kick_shown(ctx);
        }

        if released.is_some() || !down {
            let d = self.pane_drag.take().expect("drag checked above");
            let dropped = match d.over {
                Some(DropTarget::Swap(t)) => Some(Action::SwapPanes(pane, t)),
                Some(DropTarget::Edge(t, dir)) => Some(Action::MovePane(pane, t, dir)),
                Some(DropTarget::Tab(uid)) if uid != home => Some(Action::PaneToTab(pane, uid)),
                Some(DropTarget::NewTab) => Some(Action::PaneToNewTab(pane)),
                _ => None,
            };
            // Let go on nothing after opening another tab: back to the pane's own.
            if dropped.is_none() && !at_home {
                self.show_tab_uid(home);
            }
            self.kick_shown(ctx);
            return DragFrame { dropped, ..none };
        }
        if let Some(uid) = spring {
            self.show_tab_uid(uid);
        }

        let d = self.pane_drag.as_ref().expect("drag checked above");
        let mini = !area.contains(d.at);
        let s = if mini { (MINI.x / d.slot.width()).min(MINI.y / d.slot.height()).min(1.0) } else { 1.0 };
        let held = Rect::from_min_size(d.at - d.grab * s, d.slot.size() * s);
        let goals = self.drop_preview(area, at_home);
        DragFrame { goals, held: Some((pane, held)), mini, dropped: None }
    }

    /// The drop target in the title bar (or sidebar) at `at`: a tab, or the empty strip
    /// for a new tab (if the pane has company to leave behind).
    fn strip_target(&self, at: Pos2, shown: u64, home: u64) -> Option<DropTarget> {
        if let Some(&(uid, _)) = self.tab_hits.iter().find(|(_, r)| r.contains(at)) {
            // The tab already showing is only a target for a pane from another tab.
            return (uid != shown || uid != home).then_some(DropTarget::Tab(uid));
        }
        if self.files_rect.is_some_and(|r| r.contains(at)) {
            return None;
        }
        let company = self.tabs.iter().find(|t| t.uid == home).is_some_and(|t| t.root.panes().len() > 1);
        (company && self.strip_rect.is_some_and(|r| r.contains(at))).then_some(DropTarget::NewTab)
    }

    /// Where the shown tab's cards would go if the dragged pane were dropped now.
    fn drop_preview(&self, area: Rect, at_home: bool) -> HashMap<PaneId, Rect> {
        let (Some(d), Some(tab)) = (self.pane_drag.as_ref(), self.cur_tab()) else { return HashMap::new() };
        let mut tree = tab.root.clone();
        let changed = match d.over {
            // The other pane slides into the dragged one's slot.
            Some(DropTarget::Swap(t)) => return [(t, d.slot)].into(),
            Some(DropTarget::Edge(t, dir)) if at_home => tree.move_pane(d.pane, t, dir),
            Some(DropTarget::Edge(t, dir)) => tree.insert(t, dir, d.pane),
            // Leaving the tab: the others close up.
            Some(DropTarget::Tab(_) | DropTarget::NewTab) if at_home => tree.remove(d.pane),
            _ => false,
        };
        if !changed || tab.zoomed {
            return HashMap::new();
        }
        let mut out = Vec::new();
        tree.layout(area, PANE_GAP, &mut out, &mut Vec::new());
        out.into_iter().filter(|(id, _)| *id != d.pane).collect()
    }

    /// Ease the shown tab's cards to their new places (the drop preview changed).
    fn kick_shown(&mut self, ctx: &egui::Context) {
        if let Some(tab) = self.cur_tab_mut() {
            tab.motion.kick_reflow(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_and_middle() {
        let r = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0));
        assert_eq!(zone(r, Pos2::new(200.0, 100.0)), None);
        assert_eq!(zone(r, Pos2::new(20.0, 100.0)), Some(Direction::Left));
        assert_eq!(zone(r, Pos2::new(390.0, 100.0)), Some(Direction::Right));
        assert_eq!(zone(r, Pos2::new(200.0, 10.0)), Some(Direction::Up));
        assert_eq!(zone(r, Pos2::new(200.0, 190.0)), Some(Direction::Down));
        // A corner goes to the nearer edge (by fraction of the pane's size).
        assert_eq!(zone(r, Pos2::new(8.0, 190.0)), Some(Direction::Left));
    }
}
