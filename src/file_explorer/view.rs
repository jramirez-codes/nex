//! Drawing an explorer pane: the path bar, the listing, the action bar and the context menu.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, DragAndDrop, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::layout::PaneId;
use crate::ui::theme::{ACCENT, ERROR, HAIRLINE, Theme, white_alpha};
use crate::ui::widgets::IconButton;
use crate::ui::{icons, paint};

use super::fs_ops::count;
use super::input::{Command, ExplorerDrag, PointerInput};
use super::items::{ItemState, Layout, Sizes, paint_row, paint_tile, sizes};
use super::transfer::{Clip, Op, clipboard};
use super::{FileExplorer, View};

/// How long a finished operation's message stays in the action bar.
pub(super) const STATUS_TIME: Duration = Duration::from_secs(4);
const SEGMENT_TRACK: Color32 = Color32::from_black_alpha(50);
const SEGMENT_ON: Color32 = white_alpha(30);

/// How an action-bar button looks.
#[derive(Clone, Copy)]
enum Style {
    Plain,
    Primary,
    Danger,
}

/// What's being drawn this frame, shared by the parts of `paint`.
struct Frame<'a> {
    ui: &'a Ui,
    painter: &'a Painter,
    pane: PaneId,
    c: &'a Theme,
    s: Sizes,
    input: PointerInput,
    /// Files being dragged out of an explorer (this one or another), if any.
    drag: Option<std::sync::Arc<ExplorerDrag>>,
    commands: Vec<Command>,
}

impl FileExplorer {
    /// Draw the path bar, listing and action bar in `rect`, handling clicks, touch gestures,
    /// scrolling and drops.
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, pane: PaneId, c: &Theme) {
        let ctx = ui.ctx().clone();
        self.poll_jobs(&ctx);
        let input = PointerInput::read(ui, &mut self.last_pos);
        let mut f = Frame {
            ui,
            painter,
            pane,
            c,
            s: sizes(self.touch, self.zoom),
            input,
            drag: DragAndDrop::payload::<ExplorerDrag>(&ctx),
            commands: Vec::new(),
        };

        let (bar, parent_drop) = self.path_bar(&mut f, rect);
        let clip = clipboard();
        let list_bottom = self.action_bar_area(&mut f, rect, clip.as_ref());
        let list = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom() + 4.0), Pos2::new(rect.right(), list_bottom));
        if list.is_positive() {
            self.listing(&mut f, list, parent_drop, clip.as_ref());
        }
        let commands = std::mem::take(&mut f.commands);
        self.apply(&ctx, commands);
        self.delete_dialog(&ctx, pane, c);
    }

    /// "Permanently delete …?" with Cancel and Delete. Enter confirms; Escape or a tap
    /// outside cancels. Buttons are sized for fingers on touch screens.
    fn delete_dialog(&mut self, ctx: &egui::Context, pane: PaneId, c: &Theme) {
        let Some(paths) = &self.confirm_delete else { return };
        let s = sizes(self.touch, 1.0);
        let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
        let has_folder = paths.iter().any(|p| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()));
        let title = match paths.as_slice() {
            [one] => format!("Permanently delete “{}”?", name(one)),
            many => format!("Permanently delete {}?", count(many.len())),
        };
        let mut body = String::new();
        if paths.len() > 1 {
            for p in paths.iter().take(5) {
                body.push_str(&format!("•  {}\n", name(p)));
            }
            if paths.len() > 5 {
                body.push_str(&format!("…and {} more\n", paths.len() - 5));
            }
            body.push('\n');
        }
        body.push_str(if has_folder {
            "Folders are deleted with everything inside them. Nothing goes to the trash, and this can't be undone."
        } else {
            "Nothing goes to the trash, and this can't be undone."
        });

        let (mut confirm, mut cancel) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        let touch = self.touch;
        let frame = egui::Frame::popup(&ctx.style()).fill(c.card).corner_radius(12).inner_margin(18.0);
        let modal = egui::Modal::new(Id::new(("files-delete", pane))).frame(frame).show(ctx, |ui| {
            ui.set_width(340.0f32.min(ctx.content_rect().width() - 48.0));
            ui.label(egui::RichText::new(title).size(s.font + 3.0).strong().color(c.text_bright));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(body).size(s.font).color(c.text));
            ui.add_space(16.0);
            let button = |label: &str, fill: Color32, text: Color32| {
                egui::Button::new(egui::RichText::new(label).size(s.font).color(text))
                    .fill(fill)
                    .corner_radius(s.button / 2.0)
                    .min_size(Vec2::new(if touch { 110.0 } else { 88.0 }, s.button))
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                confirm |= ui.add(button("Delete", ERROR, Color32::WHITE)).clicked();
                ui.add_space(6.0);
                cancel |= ui.add(button("Cancel", Color32::from_white_alpha(18), c.text_bright)).clicked();
            });
        });
        cancel |= modal.backdrop_response.clicked();
        if cancel {
            self.cancel_delete();
        } else if confirm {
            self.confirm_delete(ctx);
        }
    }

    /// The top bar: up button, current directory, select toggle and the list | grid control.
    /// Returns the bar's rect and whether dragged files are hovering the up button.
    fn path_bar(&mut self, f: &mut Frame, rect: Rect) -> (Rect, bool) {
        let (ui, painter, c, pane) = (f.ui, f.painter, f.c, f.pane);
        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), f.s.bar));
        let scale = f.s.scale;
        let btn = f.s.bar - 8.0 * scale;
        let icon_rect = |r: Rect| Rect::from_center_size(r.center(), Vec2::splat(r.width().min(26.0 * scale) - 10.0 * scale));

        // Up button (also a drop target for the parent folder).
        let up_r = Rect::from_center_size(Pos2::new(bar.left() + btn / 2.0 + 1.0, bar.center().y), Vec2::splat(btn));
        let can_go_up = self.dir.parent().is_some();
        let parent_drop = f.drag.is_some() && can_go_up && f.input.pos.is_some_and(|p| up_r.contains(p));
        if IconButton::new(up_r, Id::new(("files-up", pane)), c.text_dim, c.text_bright)
            .icon_rect(icon_rect(up_r))
            .tooltip("Parent folder  (Backspace)")
            .enabled(can_go_up)
            .active(parent_drop)
            .show(ui, painter, icons::icon_chevron_up)
        {
            self.up();
        }

        // View toggle: a two-segment control (list | grid).
        let seg = Rect::from_min_max(Pos2::new(bar.right() - btn * 2.0 - 4.0, up_r.top()), Pos2::new(bar.right(), up_r.bottom()));
        painter.rect_filled(seg, 7, SEGMENT_TRACK);
        for (k, view) in [View::List, View::Grid].into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(seg.left() + 2.0 + k as f32 * btn, seg.top() + 2.0), Vec2::new(btn, btn - 4.0));
            let on = self.view == view;
            if on {
                painter.rect_filled(r, 6, SEGMENT_ON);
            }
            let (tip, icon): (&str, fn(&Painter, Rect, Color32)) = match view {
                View::List => ("List view  (Ctrl+G)", icons::icon_list),
                View::Grid => ("Grid view  (Ctrl+G)", icons::icon_grid),
            };
            let resp = ui.interact(r, Id::new(("files-view", pane, k)), Sense::click());
            let col = if on || resp.hovered() { c.text_bright } else { c.text_dim };
            icon(painter, Rect::from_center_size(r.center(), Vec2::splat(12.0 * scale)), col);
            if resp.on_hover_text(tip).clicked() && !on {
                f.commands.push(Command::ToggleView);
            }
        }

        // Selection-mode toggle (mainly for touch screens; a long press also starts it).
        let sel_r = Rect::from_center_size(Pos2::new(seg.left() - 6.0 - btn / 2.0, bar.center().y), Vec2::splat(btn));
        let selecting = self.selecting;
        if IconButton::new(sel_r, Id::new(("files-select", pane)), c.text_dim, c.text_bright)
            .icon_rect(icon_rect(sel_r))
            .tooltip("Select items")
            .active(selecting)
            .show(ui, painter, |p, r, col| icons::icon_check(p, r.shrink(1.0), if selecting { ACCENT } else { col }, selecting))
        {
            f.commands.push(if selecting { Command::ClearSelection } else { Command::SelectMode });
        }

        let path_left = up_r.right() + 8.0;
        let font = FontId::proportional(f.s.font - 0.5);
        paint::faded_text(painter, Pos2::new(path_left, bar.center().y), &self.title(), font, c.text_dim, sel_r.left() - path_left - 6.0, 20.0);
        painter.line_segment([Pos2::new(rect.left(), bar.bottom()), Pos2::new(rect.right(), bar.bottom())], Stroke::new(1.0, HAIRLINE));
        (bar, parent_drop)
    }

    /// The bottom action bar, shown while selecting, while something is on the file clipboard
    /// or while an operation reports progress. Returns where the listing must end.
    fn action_bar_area(&mut self, f: &mut Frame, rect: Rect, clip: Option<&Clip>) -> f32 {
        let status = self.status.as_ref().filter(|(_, _, at)| !self.jobs.is_empty() || at.elapsed() < STATUS_TIME).cloned();
        let show = self.selecting || !self.marked.is_empty() || clip.is_some() || status.is_some();
        if !show {
            return rect.bottom();
        }
        let ab = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - f.s.action_bar), rect.max);
        self.action_bar(f, ab, clip, status.as_ref());
        if let Some((_, _, at)) = &status {
            f.ui.ctx().request_repaint_after(STATUS_TIME.saturating_sub(at.elapsed()));
        }
        ab.top() - 2.0
    }

    /// The listing: pointer gestures, scrolling, drops, items and the right-click menu.
    fn listing(&mut self, f: &mut Frame, list: Rect, parent_drop: bool, clip: Option<&Clip>) {
        let layout = self.update_layout(&f.s, list);
        self.apply_zoom_anchor(&layout);
        let resp = f.ui.interact(list, Id::new(("files-list", f.pane)), Sense::click_and_drag());
        self.track_press(f.ui.ctx(), &f.input, &layout, list);
        if resp.secondary_clicked() {
            self.right_click_select(f.input.pos.and_then(|p| layout.item_at(p, self.scroll, self.entries.len())));
        }
        self.update_scroll(f.ui, &layout, list, f.input.mods.ctrl);

        let drop_target = self.handle_drop(f, list, parent_drop);
        // Files dragged in from other applications.
        let os_hover = f.ui.input(|i| !i.raw.hovered_files.is_empty()) && f.ui.rect_contains_pointer(list);
        if drop_target.as_ref() == Some(&self.dir) || os_hover {
            f.painter.rect_filled(list, 8, ACCENT.gamma_multiply(0.06));
            f.painter.rect_stroke(list.shrink(1.0), 8, Stroke::new(1.5, ACCENT.gamma_multiply(0.7)), StrokeKind::Inside);
        }

        self.paint_items(f, &layout, list, drop_target.as_ref(), clip);
        self.context_menu(&resp, clip.is_some(), &mut f.commands);
    }

    /// Lay items out for `list`, remembering the layout for hit tests and keyboard paging.
    fn update_layout(&mut self, s: &Sizes, list: Rect) -> Layout {
        let layout = match self.view {
            View::List => Layout { list, cols: 1, item: Vec2::new(list.width(), s.row) },
            View::Grid => {
                let cols = ((list.width() / s.tile.x).floor() as usize).max(1);
                Layout { list, cols, item: Vec2::new(list.width() / cols as f32, s.tile.y) }
            }
        };
        self.layout = Some(layout);
        self.cols = layout.cols;
        self.page_items = ((list.height() / layout.item.y).floor() as usize).max(1) * layout.cols;
        layout
    }

    /// Wheel scrolling (Ctrl+wheel zooms instead), then keep the keyboard cursor in view and
    /// the scroll in range.
    fn update_scroll(&mut self, ui: &Ui, layout: &Layout, list: Rect, ctrl: bool) {
        if !ctrl && ui.rect_contains_pointer(list) {
            self.scroll -= ui.input(|i| i.raw_scroll_delta.y);
        }
        let len = self.entries.len();
        if std::mem::take(&mut self.reveal) && len > 0 {
            let top = (self.selected / layout.cols) as f32 * layout.item.y;
            if top < self.scroll {
                self.scroll = top;
            } else if top + layout.item.y > self.scroll + list.height() {
                self.scroll = top + layout.item.y - list.height();
            }
        }
        let content_h = layout.rows(len) as f32 * layout.item.y;
        self.scroll = self.scroll.clamp(0.0, (content_h - list.height()).max(0.0));
    }

    /// Files dragged out of an explorer can drop onto a folder, onto the listing (this
    /// folder), or onto the up button (the parent folder); Ctrl copies instead of moving.
    /// Returns the folder that would receive a drop now, for highlighting.
    fn handle_drop(&mut self, f: &Frame, list: Rect, parent_drop: bool) -> Option<PathBuf> {
        let (Some(drag), Some(p)) = (&f.drag, f.input.pos) else { return None };
        let target = if parent_drop {
            self.dir.parent().map(Path::to_path_buf)
        } else if list.contains(p) {
            Some(self.folder_at(p, &drag.paths).unwrap_or_else(|| self.dir.clone()))
        } else {
            None
        }?;
        let op = if f.input.mods.ctrl { Op::Copy } else { Op::Move };
        // Moving files into the folder they're already in does nothing; don't advertise it.
        let useful = op == Op::Copy || drag.paths.iter().any(|path| path.parent() != Some(target.as_path()));
        if !useful {
            return None;
        }
        if f.input.released {
            let ctx = f.ui.ctx();
            if let Some(drag) = DragAndDrop::take_payload::<ExplorerDrag>(ctx) {
                self.start_op(ctx, op, drag.paths.clone(), target.clone());
                self.clear_selection();
            }
        }
        Some(target)
    }

    /// The visible rows (or tiles), plus an error or "Empty folder" message.
    fn paint_items(&self, f: &Frame, layout: &Layout, list: Rect, drop_target: Option<&PathBuf>, clip: Option<&Clip>) {
        let (painter, c) = (f.painter, f.c);
        let len = self.entries.len();
        let message = FontId::proportional(12.5);
        if let Some(err) = &self.error {
            painter.text(list.center_top() + Vec2::new(0.0, 24.0), Align2::CENTER_CENTER, err, message, ERROR);
        } else if len == 0 {
            painter.text(list.center_top() + Vec2::new(0.0, 24.0), Align2::CENTER_CENTER, "Empty folder", message, c.text_dim);
        }

        let clip_p = painter.with_clip_rect(list);
        let hover = f
            .input
            .pos
            .filter(|_| !self.touch && f.drag.is_none() && f.ui.rect_contains_pointer(list))
            .and_then(|p| layout.item_at(p, self.scroll, len));
        let first_row = (self.scroll / layout.item.y).floor() as usize;
        let last_row = ((self.scroll + list.height()) / layout.item.y).ceil() as usize;
        let cut_paths: &[PathBuf] = clip.filter(|c| c.cut).map(|c| c.paths.as_slice()).unwrap_or(&[]);
        for i in (first_row * layout.cols)..((last_row * layout.cols).min(len)) {
            let r = layout.item_rect(i, self.scroll);
            let e = &self.entries[i];
            let state = ItemState {
                marked: self.marked.contains(&e.path),
                cursor: self.marked.is_empty() && !self.selecting && !self.touch && i == self.selected,
                hovered: hover == Some(i),
                drop: drop_target == Some(&e.path),
                dimmed: cut_paths.contains(&e.path),
                selecting: self.selecting,
            };
            match self.view {
                View::List => paint_row(&clip_p, r, e, &state, c, &f.s),
                View::Grid => paint_tile(&clip_p, r, e, &state, c, &f.s),
            }
        }
    }

    /// The right-click menu.
    fn context_menu(&self, resp: &Response, has_clip: bool, commands: &mut Vec<Command>) {
        let targets = self.targets().len();
        let single = self.marked.len() <= 1;
        let folder = single.then(|| self.entries.get(self.selected).filter(|e| e.is_dir).map(|_| self.selected)).flatten();
        let (selected, view, hidden) = (self.selected, self.view, self.show_hidden);
        resp.context_menu(|ui| {
            ui.set_min_width(180.0);
            let mut item = |ui: &mut Ui, label: egui::RichText, cmd: Command| {
                if ui.button(label).clicked() {
                    commands.push(cmd);
                    ui.close();
                }
            };
            if targets > 0 {
                if single {
                    item(ui, "Open".into(), Command::Open(selected));
                }
                item(ui, "Copy".into(), Command::Copy);
                item(ui, "Cut".into(), Command::Cut);
                if let (true, Some(f)) = (has_clip, folder) {
                    item(ui, "Paste into folder".into(), Command::PasteInto(f));
                }
                item(ui, (if targets > 1 { "Copy paths" } else { "Copy path" }).into(), Command::CopyPath);
                ui.separator();
                item(ui, egui::RichText::new("Delete…").color(ERROR), Command::Delete);
                ui.separator();
            }
            if has_clip {
                item(ui, "Paste here".into(), Command::Paste);
            }
            item(ui, "Select…".into(), Command::SelectMode);
            item(ui, "Select all".into(), Command::SelectAll);
            ui.separator();
            item(ui, (if view == View::List { "Grid view" } else { "List view" }).into(), Command::ToggleView);
            item(ui, (if hidden { "Hide hidden files" } else { "Show hidden files" }).into(), Command::ToggleHidden);
        });
    }

    /// The bottom bar: a status line plus selection and clipboard buttons.
    fn action_bar(&self, f: &mut Frame, ab: Rect, clip: Option<&Clip>, status: Option<&(String, bool, Instant)>) {
        let (ui, painter, c, s) = (f.ui, f.painter, f.c, &f.s);
        painter.line_segment([ab.left_top(), ab.right_top()], Stroke::new(1.0, HAIRLINE));
        let selecting = self.selecting || !self.marked.is_empty();
        let mut buttons: Vec<(&str, Command, Style)> = Vec::new();
        if selecting {
            buttons.push(("All", Command::SelectAll, Style::Plain));
            if !self.marked.is_empty() {
                buttons.push(("Copy", Command::Copy, Style::Plain));
                buttons.push(("Cut", Command::Cut, Style::Plain));
                buttons.push(("Delete", Command::Delete, Style::Danger));
            }
        }
        if let Some(clip) = clip {
            buttons.push((if clip.cut { "Move here" } else { "Paste here" }, Command::Paste, Style::Primary));
            if !selecting {
                buttons.push(("Cancel", Command::ClearClipboard, Style::Plain));
            }
        }
        if selecting {
            buttons.push(("Done", Command::ClearSelection, Style::Plain));
        }

        // Pill buttons, laid out from the right.
        let font = FontId::proportional(s.font - 0.5);
        let mut x = ab.right() - 4.0;
        for (k, (label, cmd, style)) in buttons.iter().enumerate().rev() {
            let w = painter.layout_no_wrap(label.to_string(), font.clone(), c.text).size().x + (if self.touch { 28.0 } else { 20.0 }) * s.scale;
            let r = Rect::from_min_size(Pos2::new(x - w, ab.center().y - s.button / 2.0), Vec2::new(w, s.button));
            if r.left() < ab.left() + 4.0 {
                break;
            }
            x = r.left() - 6.0;
            let resp = ui.interact(r, Id::new(("files-action", f.pane, k)), Sense::click());
            let hot = resp.hovered();
            let fill = match style {
                Style::Primary => ACCENT.gamma_multiply(if hot { 1.0 } else { 0.8 }),
                Style::Danger => ERROR.gamma_multiply(if hot { 0.35 } else { 0.22 }),
                Style::Plain => Color32::from_white_alpha(if hot { 34 } else { 18 }),
            };
            painter.rect_filled(r, (s.button / 2.0) as u8, fill);
            let col = match style {
                Style::Primary => Color32::WHITE,
                Style::Danger => ERROR,
                Style::Plain => c.text_bright,
            };
            painter.text(r.center(), Align2::CENTER_CENTER, label, font.clone(), col);
            if resp.clicked_by(PointerButton::Primary) {
                f.commands.push(*cmd);
            }
        }

        // Status / summary on the left.
        let (text, col) = match (status, clip) {
            (Some((msg, err, _)), _) => (msg.clone(), if *err { ERROR } else { c.text }),
            _ if !self.marked.is_empty() => (format!("{} selected", count(self.marked.len())), c.text),
            _ if self.selecting => ("Tap items to select".to_string(), c.text_dim),
            (None, Some(clip)) => (format!("{} to {}", count(clip.paths.len()), if clip.cut { "move" } else { "copy" }), c.text_dim),
            _ => (String::new(), c.text_dim),
        };
        let left = ab.left() + 8.0;
        paint::faded_text(painter, Pos2::new(left, ab.center().y), &text, font, col, x - left - 4.0, 18.0);
    }
}
