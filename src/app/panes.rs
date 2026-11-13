//! Drawing the panes of the current tab: cards, headers, terminals, explorers, search.

use std::time::Duration;

use egui::{
    Color32, CursorIcon, FontId, Id,
    Painter, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

use crate::chrome::{self, bold};
use crate::files::{self};
use crate::layout::{Axis, PaneId};
use crate::render::{self, PaintOptions};
use crate::search::Search;
use crate::terminal::GridSize;

use crate::theme::{Theme, darken};
use super::{App, HEADER_HEIGHT, PANE_GAP};
use super::actions::Action;
use super::metrics::cell_px;

const PANE_RADIUS: u8 = 12;
const BLINK_INTERVAL: Duration = Duration::from_millis(600);

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

        // Divider dragging.
        for div in &dividers {
            let id = Id::new(("divider", tab_uid, &div.path));
            let resp = ui.interact(div.rect, id, Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(match div.axis {
                    Axis::Horizontal => CursorIcon::ResizeHorizontal,
                    Axis::Vertical => CursorIcon::ResizeVertical,
                });
            }
            if resp.dragged() {
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

        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let painter = ui.painter().clone();
        let blink_on = {
            let ms = ctx.input(|i| i.time * 1000.0) as u128;
            (ms / BLINK_INTERVAL.as_millis()) % 2 == 0
        };

        for (id, card) in rects {
            let is_focused = id == focused;
            if self.browsers.contains_key(&id) {
                self.browser_pane(ui, &painter, id, card, is_focused, multi, theme);
                self.card_rects.push((id, card));
                continue;
            }
            let Some(t) = self.panes.get(&id) else { continue };

            // Card.
            let bell = t.bell_at.is_some_and(|at| at.elapsed() < Duration::from_millis(200));
            let accent = bell.then_some(self.cfg.palette[3]);
            chrome::card(&painter, card, PANE_RADIUS, theme.card, is_focused && multi, accent);

            // Header.
            let pad = self.cfg.window_padding;
            let term_rect = self.term_rect_for(card, ppp);
            let content_top = if self.cfg.pane_headers {
                let header = Rect::from_min_size(card.min, Vec2::new(card.width(), HEADER_HEIGHT));
                let title = t.title();
                let process = t.process.clone();
                let zoomed = self.cur_tab().is_some_and(|t| t.zoomed);
                let show_buttons = is_focused || ui.rect_contains_pointer(card);
                self.pane_header(ui, &painter, header, id, &title, &process, is_focused, show_buttons, zoomed, theme);
                header.bottom()
            } else {
                card.top() + pad * 0.5
            };

            let m = self.pane_metrics(id);
            let cell_px = cell_px(&ctx, m);
            let cols = ((term_rect.width() / m.width).floor() as usize).max(2);
            let lines = ((term_rect.height() / m.height).floor() as usize).max(1);

            let t = self.panes.get_mut(&id).unwrap();
            t.resize(GridSize { cols, lines }, cell_px);
            let blinking = t.term.lock().cursor_style().blinking;
            let typing = t.last_input.elapsed() < BLINK_INTERVAL;
            let cursor_visible = !is_focused || !blinking || typing || blink_on;

            // Keep search results in sync with the terminal contents.
            let search = self.search.as_mut().filter(|s| s.pane == id);
            let search = search.map(|s| {
                let term = t.term.lock();
                let jump = s.refresh(&term, t.generation);
                drop(term);
                if let Some(p) = jump {
                    t.term.lock().scroll_to_point(p);
                }
                &*s
            });
            let link = self.hover_link.as_ref().filter(|(pid, _)| *pid == id).map(|(_, l)| &l.range);

            let clip = painter.with_clip_rect(card.shrink(1.0));
            render::paint_terminal(
                &clip,
                term_rect,
                t,
                &self.cfg,
                m,
                PaintOptions {
                    focused: is_focused && ctx.input(|i| i.focused),
                    cursor_visible,
                    bg_alpha: self.cfg.background_opacity,
                    matches: search.map(|s| s.matches.as_slice()).unwrap_or_default(),
                    current_match: search.and_then(|s| s.current()),
                    link,
                },
            );
            if let Some(s) = search {
                search_bar(&painter, card, content_top, s, theme, blink_on);
            }

            // Dim unfocused splits.
            if multi && !is_focused && self.cfg.unfocused_split_opacity < 1.0 {
                let a = 1.0 - self.cfg.unfocused_split_opacity;
                painter.rect_filled(card, PANE_RADIUS, darken(self.cfg.background, 0.5).gamma_multiply(a));
            }

            // Scrollback indicator.
            let offset = t.term.lock().grid().display_offset();
            if offset > 0 {
                let label = format!("↑ {offset}");
                let galley = painter.layout_no_wrap(label, FontId::proportional(11.0), theme.text);
                let r = Rect::from_min_size(
                    Pos2::new(card.right() - galley.size().x - 22.0, content_top + 6.0),
                    galley.size() + Vec2::new(12.0, 6.0),
                );
                painter.rect_filled(r, 6, Color32::from_black_alpha(140));
                painter.galley(r.min + Vec2::new(6.0, 3.0), galley, theme.text);
            }

            self.term_rects.push((id, term_rect));
            self.card_rects.push((id, card));
        }

        // Keep blinking cursors blinking.
        let ms = ctx.input(|i| i.time * 1000.0);
        let next = BLINK_INTERVAL.as_millis() as f64 - ms % BLINK_INTERVAL.as_millis() as f64;
        ctx.request_repaint_after(Duration::from_millis(next as u64 + 1));
    }

    /// Draw a file-browser pane: card, header (with the usual split/zoom/close buttons) and
    /// the listing. It isn't added to `term_rects`, so terminal mouse handling ignores it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn browser_pane(&mut self, ui: &mut Ui, painter: &Painter, id: PaneId, card: Rect, is_focused: bool, multi: bool, theme: &Theme) {
        chrome::card(painter, card, PANE_RADIUS, theme.card, is_focused && multi, None);
        if self.cfg.pane_headers {
            let header = Rect::from_min_size(card.min, Vec2::new(card.width(), HEADER_HEIGHT));
            let (title, process) = self.pane_label(id);
            let zoomed = self.cur_tab().is_some_and(|t| t.zoomed);
            let show_buttons = is_focused || ui.rect_contains_pointer(card);
            self.pane_header(ui, painter, header, id, &title, &process, is_focused, show_buttons, zoomed, theme);
        }
        let content = self.term_rect_for(card, ui.ctx().pixels_per_point());
        if ui.rect_contains_pointer(content) && ui.input(|i| i.pointer.primary_pressed()) {
            self.pending.push(Action::FocusPane(id));
        }
        let colors = files::Colors { text: theme.text, text_dim: theme.text_dim, text_bright: theme.text_bright };
        if let Some(b) = self.browsers.get_mut(&id) {
            b.paint(ui, &painter.with_clip_rect(card.shrink(1.0)), content, id, &colors);
        }
        if multi && !is_focused && self.cfg.unfocused_split_opacity < 1.0 {
            let a = 1.0 - self.cfg.unfocused_split_opacity;
            painter.rect_filled(card, PANE_RADIUS, darken(self.cfg.background, 0.5).gamma_multiply(a));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn pane_header(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        header: Rect,
        id: PaneId,
        title: &str,
        process: &str,
        focused: bool,
        show_buttons: bool,
        zoomed: bool,
        theme: &Theme,
    ) {
        let cy = header.center().y + 1.0;
        let title_color = if focused { theme.text_bright } else { theme.text.gamma_multiply(0.85) };
        let glyph = Rect::from_center_size(Pos2::new(header.left() + 20.0, cy), Vec2::splat(13.0));
        chrome::header_glyph(painter, glyph, process, title_color);

        // Buttons (right to left): close, zoom, split down, split right — shown on the
        // focused or hovered card only, keeping the other headers clean.
        let mut bx = header.right() - 20.0;
        if show_buttons {
            let mut button = |name: &str, draw: &dyn Fn(&Painter, Rect, Color32)| -> bool {
                let r = Rect::from_center_size(Pos2::new(bx, cy), Vec2::splat(22.0));
                bx -= 24.0;
                let resp = ui.interact(r, Id::new(("pane-btn", id, name)), Sense::click());
                if resp.hovered() {
                    painter.rect_filled(r, 6, Color32::from_white_alpha(16));
                }
                let c = if resp.hovered() { theme.text_bright } else { theme.text_dim };
                draw(painter, r.shrink(5.5), c);
                resp.on_hover_text(name).clicked_by(PointerButton::Primary)
            };
            if button("Close  (Ctrl+Shift+W)", &|p, r, c| chrome::icon_close(p, r, c)) {
                self.pending.push(Action::ClosePane(id));
            }
            if button("Zoom  (Ctrl+Shift+Enter)", &|p, r, c| chrome::icon_zoom(p, r, c, zoomed)) {
                self.pending.push(Action::ToggleZoomPane(id));
            }
            if button("Split down  (Ctrl+Shift+E)", &|p, r, c| chrome::icon_split(p, r, c, Axis::Vertical)) {
                self.pending.push(Action::SplitPane(id, Axis::Vertical));
            }
            if button("Split right  (Ctrl+Shift+O)", &|p, r, c| chrome::icon_split(p, r, c, Axis::Horizontal)) {
                self.pending.push(Action::SplitPane(id, Axis::Horizontal));
            }
        }

        let text_left = glyph.right() + 8.0;
        chrome::faded_text(painter, Pos2::new(text_left, cy), title, bold(13.5), title_color, bx - 4.0 - text_left, 24.0);

        // Clicking the header focuses the pane.
        if ui.rect_contains_pointer(header) && ui.input(|i| i.pointer.primary_pressed()) {
            self.pending.push(Action::FocusPane(id));
        }
    }
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
        (s.query.clone(), if s.matches.is_empty() { Color32::from_rgb(0xf2, 0x77, 0x7a) } else { theme.text })
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
