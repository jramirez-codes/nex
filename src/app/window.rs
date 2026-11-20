//! Window-level concerns: resize edges and files dropped onto the window.

use std::path::PathBuf;

use egui::{Color32, CursorIcon, FontId, Id, LayerId, Order, Rect, ResizeDirection, Stroke, StrokeKind, Vec2, ViewportCommand};

use crate::file_explorer;
use crate::layout::PaneId;
use crate::ui::theme::{ACCENT, Theme};
use crate::ui::widgets::is_maximized;

use super::App;
use super::panes::PANE_RADIUS;
use super::actions::Action;

const RESIZE_BORDER: f32 = 6.0;

impl App {
    /// Files dropped onto a pane: dragged out of a explorer onto a terminal (inserts their
    /// paths), or dropped in from another window (into an explorer: moved there, or copied with
    /// Ctrl or when the source only allows copying; onto a terminal: paths inserted). A
    /// terminal running `ssh` gets the files uploaded to its host instead (see `upload`).
    pub(super) fn file_drops(&mut self, ctx: &egui::Context, theme: &Theme) {
        let (pos, released, dropped) = ctx.input(|i| {
            let dropped: Vec<PathBuf> = i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect();
            // On release, where it was released: a touch lift can be followed by a pointer
            // move to the hidden mouse cursor.
            let pos = file_explorer::release_pos(i).or(i.pointer.interact_pos());
            (pos, i.pointer.any_released(), dropped)
        });
        let pos = pos.or(self.last_pointer);
        self.last_pointer = pos;

        // An explorer drag leaving the window goes to the compositor, so another window
        // (another nex, a file manager, a browser) can take the drop.
        if let (Some(dnd), Some(p)) = (&mut self.dnd, pos)
            && !released && !ctx.content_rect().contains(p)
                && let Some(drag) = egui::DragAndDrop::payload::<file_explorer::ExplorerDrag>(ctx) {
                    crate::platform::dnd_log(format_args!("explorer drag left the window at {p:?} (window {:?})", ctx.content_rect()));
                    // The compositor draws the icon from here on (nex can't draw outside its window).
                    if dnd.start_drag(&drag.paths, p, file_explorer::drag_image(ctx, theme, &drag)) {
                        egui::DragAndDrop::clear_payload(ctx);
                        ctx.request_repaint();
                    }
                }

        // Files dragged in from another window, over a terminal.
        if ctx.input(|i| !i.raw.hovered_files.is_empty())
            && let Some((_, card)) = pos.and_then(|p| self.terminal_card_at(p)) {
                paint_terminal_drop(ctx, card);
            }

        // Explorer → terminal, anywhere on the terminal's card (header included, which helps
        // fingers). Explorer → explorer drops are handled by the explorers.
        if egui::DragAndDrop::has_payload_of_type::<file_explorer::ExplorerDrag>(ctx)
            && let Some((id, card)) = pos.and_then(|p| self.terminal_card_at(p)) {
                if released {
                    if let Some(drag) = egui::DragAndDrop::take_payload::<file_explorer::ExplorerDrag>(ctx) {
                        self.insert_or_upload(ctx, id, drag.paths.clone());
                        self.pending.push(Action::FocusPane(id));
                    }
                } else {
                    paint_terminal_drop(ctx, card);
                }
            }

        if dropped.is_empty() {
            return;
        }
        // The pane under the drop. Only a drop with no position at all goes to the focused pane:
        // one outside every pane (between panes, or outside the window) does nothing, rather
        // than copying files somewhere nobody pointed at.
        let target = match pos {
            Some(p) => self.card_rects.iter().find(|(_, r)| r.contains(p)).map(|&(id, _)| id),
            None => self.focused_pane(),
        };
        let Some(id) = target else { return };
        if let Some(dnd) = &self.dnd {
            dnd.focus_window();
        }
        self.pending.push(Action::FocusPane(id));
        if let Some(b) = self.explorers.get_mut(&id) {
            // A move, as within the window, unless the source only allows a copy or Ctrl is held.
            let moves = self.dnd.as_ref().is_some_and(|d| d.drop_moves()) && !ctx.input(|i| crate::platform::SHORTCUTS.copy_drag(&i.modifiers));
            let op = if moves { file_explorer::Op::Move } else { file_explorer::Op::Copy };
            crate::platform::dnd_log(format_args!("{op:?} {} path(s) into explorer pane {id:?}", dropped.len()));
            b.drop_external(ctx, op, dropped, pos);
        } else {
            crate::platform::dnd_log(format_args!("typed {} path(s) into pane {id:?}", dropped.len()));
            self.insert_or_upload(ctx, id, dropped);
        }
    }

    /// The terminal pane whose card is under `pos`, with the card's rect.
    fn terminal_card_at(&self, pos: egui::Pos2) -> Option<(PaneId, Rect)> {
        self.card_rects.iter().find(|(id, r)| r.contains(pos) && self.panes.contains_key(id)).copied()
    }

    /// Type shell-quoted paths into a terminal, as if pasted.
    pub(super) fn insert_paths(&mut self, id: PaneId, paths: &[PathBuf]) {
        if let Some(t) = self.panes.get_mut(&id) {
            let text = paths.iter().map(|p| crate::platform::shell_quote(p)).collect::<Vec<_>>().join(" ");
            t.paste(&format!("{text} "));
        }
    }

    /// Frameless windows need their own resize handles at the edges.
    pub(super) fn window_resize_edges(&self, ctx: &egui::Context, rect: Rect) {
        if self.cfg.window_decorations || is_maximized(ctx) {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        let b = RESIZE_BORDER;
        let (l, r) = (pos.x < rect.left() + b, pos.x > rect.right() - b);
        let (t, bo) = (pos.y < rect.top() + b, pos.y > rect.bottom() - b);
        let dir = match (l, r, t, bo) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return,
        };
        ctx.set_cursor_icon(match dir {
            ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
            ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

/// Highlight a terminal that would receive dragged files, with a hint of what a drop does
/// (on a touch screen the finger hides most of the drag ghost).
fn paint_terminal_drop(ctx: &egui::Context, card: Rect) {
    let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("terminal-drop-target")));
    p.rect_filled(card, PANE_RADIUS, ACCENT.gamma_multiply(0.08));
    p.rect_stroke(card.shrink(1.0), PANE_RADIUS, Stroke::new(2.0_f32, ACCENT.gamma_multiply(0.8)), StrokeKind::Inside);
    let galley = p.layout_no_wrap("Drop to insert path".into(), FontId::proportional(13.0), Color32::WHITE);
    let pill = Rect::from_center_size(card.center(), galley.size() + Vec2::new(28.0, 14.0));
    p.rect_filled(pill, (pill.height() / 2.0) as u8, ACCENT.gamma_multiply(0.85));
    p.galley(pill.center() - galley.size() / 2.0, galley, Color32::WHITE);
}
