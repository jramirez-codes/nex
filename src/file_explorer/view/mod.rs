//! Drawing an explorer pane: the path bar (Back / Forward / Up, breadcrumbs or the location
//! field), the listing, the action bar with job progress, the context menu and dialogs.

pub(super) mod items;
pub(super) mod properties;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, DragAndDrop, FontId, Id, Key, Painter, PointerButton, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::layout::PaneId;
use crate::platform::SHORTCUTS;
use crate::ui::file_icons::paint_glyph;
use crate::ui::theme::{ACCENT, ERROR, HAIRLINE, Theme, white_alpha};
use crate::ui::widgets::IconButton;
use crate::ui::{icons, paint};

use crate::file_explorer::fs::ops::count;
use crate::file_explorer::input::{Command, ExplorerDrag, PointerInput};
use crate::file_explorer::view::items::{ItemState, Layout, Sizes, name_rect, paint_row, paint_tile, sizes};
use crate::file_explorer::fs::transfer::{Clip, JobSummary, Op, clipboard};
use crate::file_explorer::{FileExplorer, View};

/// How long a finished operation's message stays in the action bar.
pub(super) const STATUS_TIME: Duration = Duration::from_secs(4);
const SEGMENT_TRACK: Color32 = Color32::from_black_alpha(50);
const SEGMENT_ON: Color32 = white_alpha(30);
/// The magnifier drawn in the type-to-find pill.
const FIND_GLYPH: char = '\u{f0349}';

/// A path-bar button: id, tooltip, enabled, what it does, icon.
type NavButton = (&'static str, String, bool, Command, fn(&Painter, Rect, Color32));

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

/// `(label, folder)` for each breadcrumb of `dir`: the root (or `~` for the home folder and
/// everything under it), then each folder down to `dir`.
fn crumbs(dir: &Path, home: Option<&Path>) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut chain: Vec<&Path> = dir.ancestors().collect();
    chain.reverse();
    for a in chain {
        if home == Some(a) && a.parent().is_some() {
            out.clear();
            out.push(("~".to_string(), a.to_path_buf()));
            continue;
        }
        let label = a.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| a.display().to_string());
        out.push((label, a.to_path_buf()));
    }
    out
}

impl FileExplorer {
    /// Draw the path bar, listing and action bar in `rect`, handling clicks, touch gestures,
    /// scrolling and drops.
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, pane: PaneId, c: &Theme) {
        let ctx = ui.ctx().clone();
        // Before polling: see `start_listing`.
        let _ = self.repaint.set(ctx.clone());
        self.poll_listing();
        self.poll_jobs(&ctx);
        self.expire_find(&ctx);
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

        // A mouse's side buttons go Back and Forward.
        if ui.rect_contains_pointer(rect) && !self.modal_open() {
            let (back, forward) = ui.input(|i| (i.pointer.button_pressed(PointerButton::Extra1), i.pointer.button_pressed(PointerButton::Extra2)));
            if back {
                f.commands.push(Command::Back);
            } else if forward {
                f.commands.push(Command::Forward);
            }
        }

        let (bar, bar_drop) = self.path_bar(&mut f, rect);
        let clip = clipboard();
        let list_bottom = self.action_bar_area(&mut f, rect, clip.as_ref());
        let list = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom() + 4.0), Pos2::new(rect.right(), list_bottom));
        if list.is_positive() {
            self.listing(&mut f, list, bar_drop, clip.as_ref());
        }
        let commands = std::mem::take(&mut f.commands);
        self.apply(&ctx, commands);
        self.delete_dialog(&ctx, pane, c);
        self.properties_dialog(&ctx, pane, c);
    }

    /// "Permanently delete …?" with Cancel and Delete. Enter confirms; Escape or a tap
    /// outside cancels. Buttons are sized for fingers on touch screens.
    fn delete_dialog(&mut self, ctx: &egui::Context, pane: PaneId, c: &Theme) {
        let Some(paths) = &self.confirm_delete else { return };
        let s = sizes(self.touch, 1.0);
        let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
        let has_folder = self.delete_has_folder;
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
            ui.label(RichText::new(title).size(s.font + 3.0).strong().color(c.text_bright));
            ui.add_space(8.0);
            ui.label(RichText::new(body).size(s.font).color(c.text));
            ui.add_space(16.0);
            let button = |label: &str, fill: Color32, text: Color32| {
                egui::Button::new(RichText::new(label).size(s.font).color(text))
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

    /// The top bar: Back / Forward / Up, the breadcrumbs (or the location field), the select
    /// toggle and the list | grid control. Returns the bar's rect and the folder dragged
    /// files would drop into from it (the Up button or a breadcrumb), if any.
    fn path_bar(&mut self, f: &mut Frame, rect: Rect) -> (Rect, Option<PathBuf>) {
        let (ui, painter, c, pane) = (f.ui, f.painter, f.c, f.pane);
        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), f.s.bar));
        let scale = f.s.scale;
        let btn = f.s.bar - 8.0 * scale;
        let icon_rect = |r: Rect| Rect::from_center_size(r.center(), Vec2::splat(r.width().min(26.0 * scale) - 10.0 * scale));
        let hovering = |r: Rect| f.drag.is_some() && f.input.pos.is_some_and(|p| r.contains(p));
        let mut bar_drop = None;

        // Back, Forward and Up. Up is also a drop target for the parent folder.
        let alt = SHORTCUTS.alt_key_name();
        let parent = self.dir.parent().map(Path::to_path_buf);
        let mut x = bar.left() + 1.0;
        let buttons: [NavButton; 3] = [
            ("files-back", format!("Back  ({alt}+Left)"), self.can_go_back(), Command::Back, icons::icon_chevron_left),
            ("files-forward", format!("Forward  ({alt}+Right)"), self.can_go_forward(), Command::Forward, icons::icon_chevron_right),
            ("files-up", "Parent folder  (Backspace)".into(), parent.is_some(), Command::Up, icons::icon_chevron_up),
        ];
        for (id, tip, enabled, cmd, icon) in buttons {
            let r = Rect::from_min_size(Pos2::new(x, bar.center().y - btn / 2.0), Vec2::splat(btn));
            x = r.right() + 1.0;
            let drop_here = cmd == Command::Up && enabled && hovering(r);
            if drop_here {
                bar_drop.clone_from(&parent);
            }
            if IconButton::new(r, Id::new((id, pane)), c.text_dim, c.text_bright)
                .icon_rect(icon_rect(r))
                .tooltip(&tip)
                .enabled(enabled)
                .active(drop_here)
                .show(ui, painter, icon)
            {
                f.commands.push(cmd);
            }
        }

        // View toggle: a two-segment control (list | grid).
        let seg_top = bar.center().y - btn / 2.0;
        let seg = Rect::from_min_max(Pos2::new(bar.right() - btn * 2.0 - 4.0, seg_top), Pos2::new(bar.right(), seg_top + btn));
        painter.rect_filled(seg, 7, SEGMENT_TRACK);
        for (k, view) in [View::List, View::Grid].into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(seg.left() + 2.0 + k as f32 * btn, seg.top() + 2.0), Vec2::new(btn, btn - 4.0));
            let on = self.view == view;
            if on {
                painter.rect_filled(r, 6, SEGMENT_ON);
            }
            let (label, icon): (&str, fn(&Painter, Rect, Color32)) = match view {
                View::List => ("List view", icons::icon_list),
                View::Grid => ("Grid view", icons::icon_grid),
            };
            let tip = format!("{label}  ({}+G)", SHORTCUTS.primary_key_name());
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

        let area = Rect::from_min_max(Pos2::new(x + 4.0 * scale, bar.top() + 4.0 * scale), Pos2::new(sel_r.left() - 6.0, bar.bottom() - 4.0 * scale));
        if area.width() > 24.0 {
            let font = FontId::proportional(f.s.font - 0.5);
            if let Some(field) = self.location.as_mut() {
                field.paint(painter, area, font, c);
            } else if let Some(target) = self.breadcrumbs(f, area, font) {
                bar_drop = Some(target);
            }
        }
        painter.line_segment([Pos2::new(rect.left(), bar.bottom()), Pos2::new(rect.right(), bar.bottom())], Stroke::new(1.0_f32, HAIRLINE));
        (bar, bar_drop)
    }

    /// One pill per folder of the path; leading ones fold into "…" when they don't fit.
    /// Clicking one goes there, and dragged files can be dropped on one. Clicking the empty
    /// space after them types a location instead. Returns the crumb dragged files are over.
    fn breadcrumbs(&mut self, f: &mut Frame, area: Rect, font: FontId) -> Option<PathBuf> {
        let (ui, painter, c, pane) = (f.ui, f.painter, f.c, f.pane);
        let k = f.s.scale;
        let (pad, sep) = (7.0 * k, 12.0 * k);
        let home = self.home();
        let all = crumbs(&self.dir, home.as_deref());
        let width = |label: &str| painter.layout_no_wrap(label.to_string(), font.clone(), c.text).size().x + pad * 2.0;
        let widths: Vec<f32> = all.iter().map(|(l, _)| width(l)).collect();
        let ellipsis_w = width("…");

        // Drop leading crumbs until the rest fit; the last one always stays.
        let mut first = 0;
        let total = |first: usize| {
            let shown: f32 = widths[first..].iter().sum::<f32>() + sep * (all.len() - first).saturating_sub(1) as f32;
            if first > 0 { shown + ellipsis_w + sep } else { shown }
        };
        while first + 1 < all.len() && total(first) > area.width() {
            first += 1;
        }
        let mut shown: Vec<(String, PathBuf, f32)> = Vec::new();
        if first > 0 {
            shown.push(("…".into(), all[first - 1].1.clone(), ellipsis_w));
        }
        shown.extend(all.iter().zip(&widths).skip(first).map(|((l, p), w)| (l.clone(), p.clone(), *w)));

        let mut x = area.left();
        let mut go = None;
        let mut drop_target = None;
        let last = shown.len().saturating_sub(1);
        for (i, (label, path, w)) in shown.iter().enumerate() {
            let current = i == last;
            let r = Rect::from_min_size(Pos2::new(x, area.top()), Vec2::new(w.min(area.right() - x), area.height()));
            let resp = ui.interact(r, Id::new(("files-crumb", pane, i)), Sense::click());
            let dropping = !current && f.drag.is_some() && f.input.pos.is_some_and(|p| r.contains(p));
            if dropping {
                painter.rect_filled(r, 6, ACCENT.gamma_multiply(0.22));
                painter.rect_stroke(r, 6, Stroke::new(1.5_f32, ACCENT), StrokeKind::Inside);
                drop_target = Some(path.clone());
            } else if resp.hovered() && !current {
                painter.rect_filled(r, 6, white_alpha(16));
            } else if current {
                painter.rect_filled(r, 6, white_alpha(9));
            }
            let col = if current { c.text_bright } else if resp.hovered() { c.text } else { c.text_dim };
            paint::faded_text(painter, Pos2::new(r.left() + pad, r.center().y), label, font.clone(), col, r.width() - pad * 2.0, 14.0);
            let resp = resp.on_hover_text(self.shown_path(path));
            if resp.clicked() && !current {
                go = Some(path.clone());
            }
            x = r.right();
            if !current {
                painter.text(Pos2::new(x + sep / 2.0, area.center().y), Align2::CENTER_CENTER, "›", font.clone(), c.text_dim.gamma_multiply(0.7));
                x += sep;
            }
        }

        // The rest of the bar: click to type a location.
        let rest = Rect::from_min_max(Pos2::new(x + 2.0, area.top()), area.max);
        if rest.width() > 4.0 {
            let tip = format!("Type a location  ({}+L)", SHORTCUTS.primary_key_name());
            let resp = ui.interact(rest, Id::new(("files-location", pane)), Sense::click()).on_hover_cursor(egui::CursorIcon::Text).on_hover_text(tip);
            if resp.clicked() {
                f.commands.push(Command::EditLocation);
            }
        }
        if let Some(dir) = go {
            self.navigate(dir);
        }
        drop_target
    }

    /// The bottom action bar, shown while selecting, while something is on the file clipboard
    /// or while an operation runs or reports. Returns where the listing must end.
    fn action_bar_area(&mut self, f: &mut Frame, rect: Rect, clip: Option<&Clip>) -> f32 {
        let jobs = self.job_summary();
        let status = self.status.as_ref().filter(|(_, _, at)| at.elapsed() < STATUS_TIME).cloned();
        let show = self.selecting || !self.marked.is_empty() || clip.is_some() || status.is_some() || jobs.is_some();
        if jobs.is_none() {
            self.shown_progress = 0.0;
        }
        if !show {
            return rect.bottom();
        }
        let ab = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - f.s.action_bar), rect.max);
        self.action_bar(f, ab, clip, status.as_ref(), jobs.as_ref());
        if let (Some((_, _, at)), None) = (&status, &jobs) {
            f.ui.ctx().request_repaint_after(STATUS_TIME.saturating_sub(at.elapsed()));
        }
        ab.top() - 2.0
    }

    /// The listing: pointer gestures, scrolling, drops, items and the right-click menu.
    fn listing(&mut self, f: &mut Frame, list: Rect, bar_drop: Option<PathBuf>, clip: Option<&Clip>) {
        let layout = self.update_layout(&f.s, list);
        self.apply_zoom_anchor(&layout);
        let resp = f.ui.interact(list, Id::new(("files-list", f.pane)), Sense::click_and_drag());
        self.track_press(f.ui.ctx(), &f.input, &layout, list, f.ui.layer_id());
        if resp.secondary_clicked() {
            let item = f.input.pos.and_then(|p| layout.item_at(p, self.scroll, self.entries.len()));
            self.right_click_select(item);
            // Look up "Open With" apps while the menu opens (this machine's apps open only
            // this machine's files).
            if let [one] = self.targets().as_slice()
                && !self.backend.is_remote()
                && self.menu_on_item && !one.is_dir() {
                    let one = one.clone();
                    self.load_open_with(f.ui.ctx(), one);
                }
        }
        self.update_scroll(f.ui, &layout, list, f.input.mods.command);

        // Files dragged in from other applications land where `drop_external` puts them: the
        // folder under the pointer, else this one.
        let os_hover = f.ui.input(|i| !i.raw.hovered_files.is_empty()) && f.ui.rect_contains_pointer(list);
        let os_target = os_hover.then(|| f.input.pos.and_then(|p| self.folder_at(p, &[])).unwrap_or_else(|| self.dir.clone()));
        let drop_target = self.handle_drop(f, list, bar_drop).or(os_target);
        if drop_target.as_ref() == Some(&self.dir) {
            f.painter.rect_filled(list, 8, ACCENT.gamma_multiply(0.06));
            f.painter.rect_stroke(list.shrink(1.0), 8, Stroke::new(1.5_f32, ACCENT.gamma_multiply(0.7)), StrokeKind::Inside);
        }

        self.paint_items(f, &layout, list, drop_target.as_ref(), clip);
        self.find_pill(f, list);
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
    /// folder), or onto the path bar (the Up button or a breadcrumb); Ctrl copies instead of
    /// moving. Returns the folder that would receive a drop now, for highlighting.
    fn handle_drop(&mut self, f: &Frame, list: Rect, bar_drop: Option<PathBuf>) -> Option<PathBuf> {
        let (Some(drag), Some(p)) = (&f.drag, f.input.pos) else { return None };
        // Files from another machine can't land here.
        if drag.origin != self.origin() {
            if f.input.released && list.contains(p) {
                let origin = drag.origin.clone();
                self.accepts_from(&origin);
            }
            return None;
        }
        let target = if bar_drop.is_some() {
            bar_drop
        } else if list.contains(p) {
            Some(self.folder_at(p, &drag.paths).unwrap_or_else(|| self.dir.clone()))
        } else {
            None
        }?;
        let op = if SHORTCUTS.copy_drag(&f.input.mods) { Op::Copy } else { Op::Move };
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

    /// The visible rows (or tiles), the rename field, and an error or "Empty folder" message.
    fn paint_items(&mut self, f: &Frame, layout: &Layout, list: Rect, drop_target: Option<&PathBuf>, clip: Option<&Clip>) {
        let (painter, c) = (f.painter, f.c);
        let len = self.entries.len();
        let message = FontId::proportional(12.5);
        if let Some(err) = &self.error {
            painter.text(list.center_top() + Vec2::new(0.0, 24.0), Align2::CENTER_CENTER, err, message, ERROR);
        } else if len == 0 {
            let text = if self.loading() { "Loading…" } else { "Empty folder" };
            painter.text(list.center_top() + Vec2::new(0.0, 24.0), Align2::CENTER_CENTER, text, message, c.text_dim);
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
        let in_home = self.home().is_some_and(|h| h == self.dir);
        let renaming = self.rename.as_ref().map(|r| r.path.clone());
        let mut rename_at = None;
        for i in (first_row * layout.cols)..((last_row * layout.cols).min(len)) {
            let r = layout.item_rect(i, self.scroll);
            let e = &self.entries[i];
            let hide_name = renaming.as_ref() == Some(&e.path);
            if hide_name {
                rename_at = Some(r);
            }
            let state = ItemState {
                marked: self.marked.contains(&e.path),
                cursor: self.marked.is_empty() && !self.selecting && !self.touch && i == self.selected,
                hovered: hover == Some(i),
                drop: drop_target == Some(&e.path),
                dimmed: cut_paths.contains(&e.path),
                selecting: self.selecting,
                in_home,
                hide_name,
            };
            match self.view {
                View::List => paint_row(&clip_p, r, e, &state, c, &f.s),
                View::Grid => paint_tile(&clip_p, r, e, &state, c, &f.s),
            }
        }
        self.rename_rect = None;
        if let (Some(item), Some(rename)) = (rename_at, self.rename.as_mut()) {
            let field = name_rect(self.view, item, &f.s, self.selecting);
            let font = FontId::proportional(f.s.font - if self.view == View::Grid { 1.0 } else { 0.0 });
            rename.field.paint(&clip_p, field, font, c);
            self.rename_rect = Some(field);
        }
    }

    /// While typing to find a name: a small pill with the query in the listing's corner.
    fn find_pill(&self, f: &Frame, list: Rect) {
        let Some(find) = &self.find else { return };
        let (painter, c, k) = (f.painter, f.c, f.s.scale);
        let font = FontId::proportional(f.s.font - 0.5);
        let col = if find.missed { ERROR } else { c.text_bright };
        let galley = painter.layout_no_wrap(find.query.clone(), font, col);
        let h = f.s.row.max(26.0 * k);
        let w = (galley.size().x + h + 14.0 * k).min(list.width() - 16.0);
        let r = Rect::from_min_size(Pos2::new(list.right() - w - 10.0, list.bottom() - h - 10.0), Vec2::new(w, h));
        painter.rect_filled(r.translate(Vec2::new(0.0, 2.0)), h / 2.0, Color32::from_black_alpha(60));
        painter.rect_filled(r, h / 2.0, c.card.gamma_multiply(2.0).to_opaque());
        painter.rect_stroke(r, h / 2.0, Stroke::new(1.0_f32, if find.missed { ERROR.gamma_multiply(0.6) } else { ACCENT.gamma_multiply(0.7) }), StrokeKind::Inside);
        paint_glyph(painter, Pos2::new(r.left() + h * 0.55, r.center().y), h * 0.5, FIND_GLYPH, c.text_dim);
        let left = r.left() + h;
        painter.with_clip_rect(r.shrink(4.0)).galley(Pos2::new(left, r.center().y - galley.size().y / 2.0), galley, col);
    }

    /// The right-click menu: what can be done with the clicked items, or (on empty space)
    /// in this folder.
    fn context_menu(&self, resp: &Response, has_clip: bool, commands: &mut Vec<Command>) {
        let on_item = self.menu_on_item || !self.marked.is_empty();
        let targets = if on_item { self.targets() } else { Vec::new() };
        let index = |p: &PathBuf| self.entries.iter().position(|e| &e.path == p);
        let single = match targets.as_slice() {
            [one] => index(one).map(|i| (i, self.entries[i].is_dir, one.clone())),
            _ => None,
        };
        let folder = single.as_ref().filter(|(_, dir, _)| *dir).map(|(i, _, _)| *i);
        let file_apps = single
            .as_ref()
            .filter(|(_, dir, _)| !dir)
            .and_then(|(_, _, path)| self.open_with.as_ref().filter(|o| &o.path == path))
            .map(|o| o.apps.clone());
        let (view, hidden, n) = (self.view, self.show_hidden, targets.len());
        let mut picked = None;
        resp.context_menu(|ui| {
            ui.set_min_width(200.0);
            let item = |ui: &mut Ui, label: RichText, cmd: Command, picked: &mut Option<Command>| {
                if ui.button(label).clicked() {
                    *picked = Some(cmd);
                    ui.close();
                }
            };
            if n > 0 {
                if let Some((i, _, _)) = single {
                    item(ui, "Open".into(), Command::Open(i), &mut picked);
                }
                if let Some(apps) = &file_apps {
                    ui.menu_button("Open With", |ui| {
                        ui.set_min_width(180.0);
                        match apps.lock().ok().as_deref().and_then(Option::as_ref) {
                            None => {
                                ui.weak("Looking for apps…");
                            }
                            Some(list) if list.is_empty() => {
                                ui.weak("No apps found");
                            }
                            Some(list) => {
                                for (k, app) in list.iter().enumerate() {
                                    let label = if app.default { RichText::new(format!("{}  (default)", app.name)).strong() } else { app.name.as_str().into() };
                                    item(ui, label, Command::OpenWith(k), &mut picked);
                                }
                            }
                        }
                    });
                }
                if folder.is_some() {
                    item(ui, "Open Terminal Here".into(), Command::OpenTerminal(folder), &mut picked);
                }
                ui.separator();
                item(ui, "Copy".into(), Command::Copy, &mut picked);
                item(ui, "Cut".into(), Command::Cut, &mut picked);
                if let (true, Some(f)) = (has_clip, folder) {
                    item(ui, "Paste into folder".into(), Command::PasteInto(f), &mut picked);
                }
                ui.separator();
                if single.is_some() {
                    item(ui, "Rename…".into(), Command::Rename, &mut picked);
                }
                item(ui, (if n > 1 { "Copy paths" } else { "Copy path" }).into(), Command::CopyPath, &mut picked);
                if single.is_some() {
                    item(ui, "Properties…".into(), Command::Properties(false), &mut picked);
                }
                ui.separator();
                item(ui, RichText::new("Delete…").color(ERROR), Command::Delete, &mut picked);
                return;
            }
            if has_clip {
                item(ui, "Paste here".into(), Command::Paste, &mut picked);
                ui.separator();
            }
            item(ui, "Create Folder…".into(), Command::NewFolder, &mut picked);
            item(ui, "Create Empty File…".into(), Command::NewFile, &mut picked);
            item(ui, "Open Terminal Here".into(), Command::OpenTerminal(None), &mut picked);
            ui.separator();
            item(ui, "Select…".into(), Command::SelectMode, &mut picked);
            item(ui, "Select all".into(), Command::SelectAll, &mut picked);
            ui.separator();
            item(ui, (if view == View::List { "Grid view" } else { "List view" }).into(), Command::ToggleView, &mut picked);
            item(ui, (if hidden { "Hide hidden files" } else { "Show hidden files" }).into(), Command::ToggleHidden, &mut picked);
            ui.separator();
            item(ui, "Folder Properties…".into(), Command::Properties(true), &mut picked);
        });
        commands.extend(picked);
    }

    /// The bottom bar: a status line (or a running job's progress) plus selection, clipboard
    /// and Cancel buttons.
    fn action_bar(&mut self, f: &mut Frame, ab: Rect, clip: Option<&Clip>, status: Option<&(String, bool, Instant)>, jobs: Option<&JobSummary>) {
        let (ui, painter, c, s) = (f.ui, f.painter, f.c, &f.s);
        painter.line_segment([ab.left_top(), ab.right_top()], Stroke::new(1.0_f32, HAIRLINE));
        if let Some(jobs) = jobs {
            self.progress_bar(ui, painter, ab, jobs.fraction, s.scale);
        }
        let selecting = self.selecting || !self.marked.is_empty();
        let mut buttons: Vec<(&str, Command, Style)> = Vec::new();
        if selecting {
            buttons.push(("All", Command::SelectAll, Style::Plain));
            if !self.marked.is_empty() {
                buttons.push(("Copy", Command::Copy, Style::Plain));
                buttons.push(("Cut", Command::Cut, Style::Plain));
                if self.marked.len() == 1 {
                    buttons.push(("Rename", Command::Rename, Style::Plain));
                }
                buttons.push(("Delete", Command::Delete, Style::Danger));
            }
        }
        if jobs.is_some() {
            buttons.push(("Cancel", Command::CancelJobs, Style::Plain));
        } else if let Some(clip) = clip {
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
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, *label));
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

        // Status / progress / summary on the left.
        let left = ab.left() + 8.0;
        let room = x - left - 4.0;
        if let Some(jobs) = jobs {
            let w = painter.layout_no_wrap(jobs.text.clone(), font.clone(), c.text).size().x.min(room);
            paint::faded_text(painter, Pos2::new(left, ab.center().y), &jobs.text, font.clone(), c.text, room, 18.0);
            let rest = room - w - 12.0;
            if rest > 30.0 && !jobs.current.is_empty() {
                paint::faded_text(painter, Pos2::new(left + w + 12.0, ab.center().y), &jobs.current, font, c.text_dim, rest, 18.0);
            }
            return;
        }
        let (text, col) = match (status, clip) {
            (Some((msg, err, _)), _) => (msg.clone(), if *err { ERROR } else { c.text }),
            _ if !self.marked.is_empty() => (format!("{} selected", count(self.marked.len())), c.text),
            _ if self.selecting => ("Tap items to select".to_string(), c.text_dim),
            (None, Some(clip)) => (format!("{} to {}", count(clip.paths.len()), if clip.cut { "move" } else { "copy" }), c.text_dim),
            _ => (String::new(), c.text_dim),
        };
        paint::faded_text(painter, Pos2::new(left, ab.center().y), &text, font, col, room, 18.0);
    }

    /// A thin accent bar along the action bar's top edge, easing toward `fraction`.
    fn progress_bar(&mut self, ui: &Ui, painter: &Painter, ab: Rect, fraction: f32, scale: f32) {
        if fraction < self.shown_progress {
            // A new job started from zero: don't animate backwards.
            self.shown_progress = fraction;
        } else {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            self.shown_progress += (fraction - self.shown_progress) * (dt * 10.0).min(1.0);
        }
        if (fraction - self.shown_progress).abs() > 0.002 {
            ui.ctx().request_repaint();
        }
        let h = 3.0 * scale;
        let track = Rect::from_min_max(Pos2::new(ab.left(), ab.top() - h / 2.0), Pos2::new(ab.right(), ab.top() + h / 2.0));
        painter.rect_filled(track, h / 2.0, white_alpha(14));
        let fill = Rect::from_min_size(track.min, Vec2::new(track.width() * self.shown_progress, h));
        if fill.width() > 0.5 {
            painter.rect_filled(fill.expand2(Vec2::new(0.0, 2.0 * scale)), h, ACCENT.gamma_multiply(0.18));
            painter.rect_filled(fill, h / 2.0, ACCENT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breadcrumbs_fold_home_into_a_tilde() {
        let names = |dir: &str, home: Option<&str>| -> Vec<String> { crumbs(Path::new(dir), home.map(Path::new)).into_iter().map(|c| c.0).collect() };
        assert_eq!(names("/usr/share/fonts", Some("/home/me")), ["/", "usr", "share", "fonts"]);
        assert_eq!(names("/home/me/code/nex", Some("/home/me")), ["~", "code", "nex"]);
        assert_eq!(names("/home/me", Some("/home/me")), ["~"]);
        assert_eq!(names("/home", Some("/home/me")), ["/", "home"]);
        assert_eq!(crumbs(Path::new("/home/me/code"), Some(Path::new("/home/me")))[1].1, PathBuf::from("/home/me/code"));
    }
}
