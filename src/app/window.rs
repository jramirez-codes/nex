//! Window-level concerns: resize edges and files dropped onto the window.

use std::path::PathBuf;

use egui::{CursorIcon, Rect, ResizeDirection, ViewportCommand};

use crate::file_explorer;
use crate::layout::PaneId;
use crate::ui::widgets::is_maximized;

use super::App;
use super::actions::Action;

const RESIZE_BORDER: f32 = 6.0;

impl App {
    /// Files dropped onto a pane: dragged out of a explorer onto a terminal (inserts their
    /// paths), or dropped in from another application (into a explorer: copied there; onto a
    /// terminal: paths inserted).
    pub(super) fn file_drops(&mut self, ctx: &egui::Context) {
        let (pos, released, dropped) = ctx.input(|i| {
            let dropped: Vec<PathBuf> = i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect();
            (i.pointer.interact_pos(), i.pointer.any_released(), dropped)
        });
        let pos = pos.or(self.last_pointer);
        self.last_pointer = pos;

        // Explorer → terminal. (Explorer → explorer drops are handled by the explorers.)
        if released && egui::DragAndDrop::has_payload_of_type::<file_explorer::ExplorerDrag>(ctx) {
            if let Some((id, _)) = pos.and_then(|p| self.terminal_at(p)) {
                if let Some(drag) = egui::DragAndDrop::take_payload::<file_explorer::ExplorerDrag>(ctx) {
                    self.insert_paths(id, &drag.paths);
                    self.pending.push(Action::FocusPane(id));
                }
            }
        }

        if dropped.is_empty() {
            return;
        }
        // The pane under the pointer, else the focused one.
        let target = pos
            .and_then(|p| self.card_rects.iter().find(|(_, r)| r.contains(p)).map(|&(id, _)| id))
            .or_else(|| self.focused_pane());
        let Some(id) = target else { return };
        if let Some(b) = self.explorers.get_mut(&id) {
            b.drop_external(ctx, dropped, pos);
        } else {
            self.insert_paths(id, &dropped);
        }
    }

    /// Type shell-quoted paths into a terminal, as if pasted.
    pub(super) fn insert_paths(&mut self, id: PaneId, paths: &[PathBuf]) {
        if let Some(t) = self.panes.get_mut(&id) {
            let text = paths.iter().map(|p| file_explorer::shell_quote(p)).collect::<Vec<_>>().join(" ");
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
