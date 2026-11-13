//! Drawing the panes of the current tab: cards, headers, terminals, file explorers, search.

use std::time::Duration;

use egui::{Color32, CursorIcon, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::layout::{Axis, Divider, PaneId};
use crate::render::{self, PaintOptions};
use crate::search::Search;
use crate::terminal::GridSize;
use crate::ui::theme::{ERROR, Theme, darken};
use crate::ui::widgets::{IconButton, IconFn, pressed_in};
use crate::ui::{icons, paint};

use super::actions::Action;
use super::metrics::cell_px;
use super::{App, HEADER_HEIGHT, PANE_GAP};

const PANE_RADIUS: u8 = 12;
const BLINK_INTERVAL: Duration = Duration::from_millis(600);
/// How long a pane's border flashes after the terminal bell.
const BELL_FLASH: Duration = Duration::from_millis(200);

/// One pane being drawn this frame.
struct PaneView {
    id: PaneId,
    /// The whole card, header included.
    card: Rect,
    focused: bool,
    /// The tab shows more than one pane (focus is highlighted, the rest dimmed).
    multi: bool,
}

impl App {
    pub(super) fn panes_area(&mut self, ui: &mut Ui, area: Rect, theme: &Theme) {
        self.term_rects.clear();
        self.card_rects.clear();
        let Some(tab) = self.cur_tab() else { return };
        let tab_uid = tab.uid;

        let mut rects = Vec::new();
        let mut dividers = Vec::new();
        if tab.zoomed {
            rects.push((tab.focused, area));
        } else {
            tab.root.layout(area, PANE_GAP, &mut rects, &mut dividers);
        }
        let focused = tab.focused;
        let multi = rects.len() > 1;
        for (id, t) in &self.panes {
            t.set_visible(rects.iter().any(|(r, _)| r == id));
        }
        self.drag_dividers(ui, tab_uid, &dividers);

        let blink_on = {
            let ms = ui.input(|i| i.time * 1000.0) as u128;
            (ms / BLINK_INTERVAL.as_millis()) % 2 == 0
        };
        for (id, card) in rects {
            let pane = PaneView { id, card, focused: id == focused, multi };
            if self.explorers.contains_key(&id) {
                self.explorer_pane(ui, &pane, theme);
            } else if self.panes.contains_key(&id) {
                self.terminal_pane(ui, &pane, theme, blink_on);
            } else {
                continue;
            }
            self.card_rects.push((id, card));
        }

        // Keep blinking cursors blinking.
        let ms = ui.input(|i| i.time * 1000.0);
        let next = BLINK_INTERVAL.as_millis() as f64 - ms % BLINK_INTERVAL.as_millis() as f64;
        ui.ctx().request_repaint_after(Duration::from_millis(next as u64 + 1));
    }

    /// Dragging the gaps between split panes resizes them.
    fn drag_dividers(&mut self, ui: &Ui, tab_uid: u64, dividers: &[Divider]) {
        for div in dividers {
            let resp = ui.interact(div.rect, Id::new(("divider", tab_uid, &div.path)), Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(match div.axis {
                    Axis::Horizontal => CursorIcon::ResizeHorizontal,
                    Axis::Vertical => CursorIcon::ResizeVertical,
                });
            }
            if !resp.dragged() {
                continue;
            }
            if let Some(pos) = resp.interact_pointer_pos() {
                let ratio = match div.axis {
                    Axis::Horizontal => (pos.x - div.parent.left()) / div.parent.width(),
                    Axis::Vertical => (pos.y - div.parent.top()) / div.parent.height(),
                };
                if let Some(r) = self.cur_tab_mut().and_then(|t| t.root.ratio_mut(&div.path)) {
                    *r = ratio.clamp(0.08, 0.92);
                }
            }
        }
    }

    /// The card and (if enabled) header every pane has. Returns the top of the content area.
    fn pane_frame(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme, accent: Option<Color32>) -> f32 {
        let painter = ui.painter();
        paint::card(painter, pane.card, PANE_RADIUS, theme.card, pane.focused && pane.multi, accent);
        if !self.cfg.pane_headers {
            return pane.card.top() + self.cfg.window_padding * 0.5;
        }
        let header = Rect::from_min_size(pane.card.min, Vec2::new(pane.card.width(), HEADER_HEIGHT));
        self.pane_header(ui, header, pane, theme);
        header.bottom()
    }

    /// Unfocused splits are dimmed (`unfocused-split-opacity`).
    fn dim_unfocused(&self, painter: &Painter, pane: &PaneView) {
        let opacity = self.cfg.unfocused_split_opacity;
        if pane.multi && !pane.focused && opacity < 1.0 {
            painter.rect_filled(pane.card, PANE_RADIUS, darken(self.cfg.background, 0.5).gamma_multiply(1.0 - opacity));
        }
    }

    fn terminal_pane(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme, blink_on: bool) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter();
        let id = pane.id;
        let bell = self.panes.get(&id).is_some_and(|t| t.bell_at.is_some_and(|at| at.elapsed() < BELL_FLASH));
        let accent = bell.then_some(self.cfg.palette[3]);
        let content_top = self.pane_frame(ui, pane, theme, accent);

        let term_rect = self.term_rect_for(pane.card, ctx.pixels_per_point());
        let m = self.pane_metrics(id);
        let cell_px = cell_px(&ctx, m);
        let cols = ((term_rect.width() / m.width).floor() as usize).max(2);
        let lines = ((term_rect.height() / m.height).floor() as usize).max(1);
        let Some(t) = self.panes.get_mut(&id) else { return };
        t.resize(GridSize { cols, lines }, cell_px);
        let typing = t.last_input.elapsed() < BLINK_INTERVAL;

        // Keep search results in sync with the terminal contents.
        let search = self.search.as_mut().filter(|s| s.pane == id).map(|s| {
            let jump = s.refresh(&t.term.lock(), t.generation);
            if let Some(p) = jump {
                t.term.lock().scroll_to_point(p);
            }
            &*s
        });
        let link = self.hover_link.as_ref().filter(|(pid, _)| *pid == id).map(|(_, l)| &l.range);
        // Taken after the search refresh, which may scroll to a match.
        let snap = t.snapshot();
        let cursor_visible = !pane.focused || !snap.cursor_blinking || typing || blink_on;
        let offset = snap.display_offset;

        render::paint_terminal(
            &painter.with_clip_rect(pane.card.shrink(1.0)),
            term_rect,
            snap,
            &mut self.glyphs,
            &self.cfg,
            m,
            PaintOptions {
                focused: pane.focused && ctx.input(|i| i.focused),
                cursor_visible,
                bg_alpha: self.cfg.background_opacity,
                matches: search.map(|s| s.matches.as_slice()).unwrap_or_default(),
                current_match: search.and_then(|s| s.current()),
                link,
            },
        );
        if let Some(s) = search {
            search_bar(painter, pane.card, content_top, s, theme, blink_on);
        }
        self.dim_unfocused(painter, pane);

        if offset > 0 {
            scrollback_badge(painter, pane.card, content_top, offset, theme);
        }
        self.term_rects.push((id, term_rect));
    }

    /// A file-explorer pane: the usual card and header, and the listing. It isn't added to
    /// `term_rects`, so terminal mouse handling ignores it.
    fn explorer_pane(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme) {
        self.pane_frame(ui, pane, theme, None);
        let content = self.term_rect_for(pane.card, ui.ctx().pixels_per_point());
        if pressed_in(ui, content) {
            self.pending.push(Action::FocusPane(pane.id));
        }
        if let Some(b) = self.explorers.get_mut(&pane.id) {
            b.paint(ui, &ui.painter().with_clip_rect(pane.card.shrink(1.0)), content, pane.id, theme);
        }
        self.dim_unfocused(ui.painter(), pane);
    }

    /// Process glyph, title, and (on the focused or hovered card only, keeping the other
    /// headers clean) close / zoom / split buttons. Clicking the header focuses the pane.
    fn pane_header(&mut self, ui: &Ui, header: Rect, pane: &PaneView, theme: &Theme) {
        let painter = ui.painter();
        let id = pane.id;
        let (title, process) = self.pane_label(id);
        let zoomed = self.cur_tab().is_some_and(|t| t.zoomed);
        let show_buttons = pane.focused || ui.rect_contains_pointer(pane.card);

        let cy = header.center().y + 1.0;
        let title_color = if pane.focused { theme.text_bright } else { theme.text.gamma_multiply(0.85) };
        let glyph = Rect::from_center_size(Pos2::new(header.left() + 20.0, cy), Vec2::splat(13.0));
        icons::header_glyph(painter, glyph, &process, title_color);

        // Buttons, laid out right to left.
        let mut bx = header.right() - 20.0;
        if show_buttons {
            let buttons: [(&str, Action, &IconFn<'_>); 4] = [
                ("Close  (Ctrl+Shift+W)", Action::ClosePane(id), &icons::icon_close),
                ("Zoom  (Ctrl+Shift+Enter)", Action::ToggleZoomPane(id), &|p, r, c| icons::icon_zoom(p, r, c, zoomed)),
                ("Split down  (Ctrl+Shift+E)", Action::SplitPane(id, Axis::Vertical), &|p, r, c| icons::icon_split(p, r, c, Axis::Vertical)),
                ("Split right  (Ctrl+Shift+O)", Action::SplitPane(id, Axis::Horizontal), &|p, r, c| icons::icon_split(p, r, c, Axis::Horizontal)),
            ];
            for (tip, action, draw) in buttons {
                let r = Rect::from_center_size(Pos2::new(bx, cy), Vec2::splat(22.0));
                bx -= 24.0;
                if IconButton::new(r, Id::new(("pane-btn", id, tip)), theme.text_dim, theme.text_bright)
                    .icon_rect(r.shrink(5.5))
                    .tooltip(tip)
                    .show(ui, painter, draw)
                {
                    self.pending.push(action);
                }
            }
        }

        let text_left = glyph.right() + 8.0;
        paint::faded_text(painter, Pos2::new(text_left, cy), &title, paint::bold(13.5), title_color, bx - 4.0 - text_left, 24.0);
        if pressed_in(ui, header) {
            self.pending.push(Action::FocusPane(id));
        }
    }
}

/// "↑ 42" in the top-right corner while scrolled back.
fn scrollback_badge(painter: &Painter, card: Rect, top: f32, offset: usize, theme: &Theme) {
    let galley = painter.layout_no_wrap(format!("↑ {offset}"), FontId::proportional(11.0), theme.text);
    let r = Rect::from_min_size(Pos2::new(card.right() - galley.size().x - 22.0, top + 6.0), galley.size() + Vec2::new(12.0, 6.0));
    painter.rect_filled(r, 6, Color32::from_black_alpha(140));
    painter.galley(r.min + Vec2::new(6.0, 3.0), galley, theme.text);
}

/// Ghostty-style floating search field in the top-right of the pane.
fn search_bar(painter: &Painter, card: Rect, top: f32, s: &Search, theme: &Theme, caret_on: bool) {
    let w = 300.0_f32.min(card.width() - 24.0);
    let r = Rect::from_min_size(Pos2::new(card.right() - w - 12.0, top + 4.0), Vec2::new(w, 30.0));
    painter.rect_filled(r.translate(Vec2::new(0.0, 2.0)), 8, Color32::from_black_alpha(80));
    painter.rect_filled(r, 8, Color32::from_rgb(0x2a, 0x27, 0x30));
    painter.rect_stroke(r, 8, Stroke::new(1.0, Color32::from_white_alpha(40)), StrokeKind::Inside);

    // Magnifier icon.
    let c = Pos2::new(r.left() + 16.0, r.center().y - 1.0);
    painter.circle_stroke(c, 4.5, Stroke::new(1.4, theme.text_dim));
    painter.line_segment([c + Vec2::new(3.2, 3.2), c + Vec2::new(6.5, 6.5)], Stroke::new(1.4, theme.text_dim));

    let count = match (s.query.is_empty(), s.index) {
        (true, _) => String::new(),
        (false, Some(i)) => format!("{}/{}", i + 1, s.matches.len()),
        (false, None) => "0/0".into(),
    };
    let font = FontId::proportional(12.5);
    let count_galley = painter.layout_no_wrap(count, font.clone(), theme.text_dim);
    let count_x = r.right() - 10.0 - count_galley.size().x;
    painter.galley(Pos2::new(count_x, r.center().y - count_galley.size().y / 2.0), count_galley, theme.text_dim);

    let text_rect = Rect::from_min_max(Pos2::new(r.left() + 30.0, r.top()), Pos2::new(count_x - 8.0, r.bottom()));
    let clip = painter.with_clip_rect(text_rect);
    let (text, color) = if s.query.is_empty() {
        ("Search…".to_string(), theme.text_dim.gamma_multiply(0.7))
    } else {
        (s.query.clone(), if s.matches.is_empty() { ERROR } else { theme.text })
    };
    let galley = painter.layout_no_wrap(text, font, color);
    // Keep the end of long queries visible.
    let tx = text_rect.left().min(text_rect.right() - galley.size().x - 3.0);
    let caret_x = if s.query.is_empty() { text_rect.left() } else { tx + galley.size().x + 1.0 };
    clip.galley(Pos2::new(tx, r.center().y - galley.size().y / 2.0), galley, color);
    if caret_on {
        clip.line_segment(
            [Pos2::new(caret_x, r.center().y - 7.0), Pos2::new(caret_x, r.center().y + 7.0)],
            Stroke::new(1.2, theme.text),
        );
    }
}
