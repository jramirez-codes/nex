//! Drawing the panes of the current tab: cards, headers, terminals, file explorers, search.
//!
//! An idle window costs nothing here: a terminal's shapes are rebuilt only when its
//! [`FrameKey`] changes, the lists a frame needs are kept between frames ([`DrawBuffers`]), and
//! the next frame is scheduled for the cursor's blink only while something is blinking.

use std::time::Duration;

use egui::emath::TSTransform;
use egui::epaint::Shadow;
use egui::{Color32, CursorIcon, FontId, Id, Painter, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::layout::{Axis, Divider, PaneId, PaneKind};
use crate::render::{self, FrameKey, PaintOptions};
use crate::terminal::GridSize;
use crate::terminal::search::Search;
use crate::ui::theme::{ERROR, HOVER, Theme, darken};
use crate::ui::widgets::{HoverShape, IconButton, IconFn, pressed_in};
use crate::ui::{icons, paint};

use crate::app::actions::Action;
use crate::app::animation::handoff::{FocusedCard, HeaderLabel};
use crate::app::animation::pane_motion::Ghost;
use crate::app::animation::tab_switch::Shown;
use crate::app::input::keymap;
use crate::app::ssh::mic::MicState;
use crate::app::workspace::metrics::cell_px;
use crate::app::workspace::pane_drag::DragFrame;
use crate::app::workspace::panes::PaneRequest;
use crate::app::{App, PANE_GAP};

pub(in crate::app) const PANE_RADIUS: u8 = 12;
/// A pane header button on desktop, and how far its icon sits inside it.
const DESKTOP_BUTTON: f32 = 28.0;
const DESKTOP_INSET: f32 = 4.5;
const BLINK_INTERVAL: Duration = Duration::from_millis(600);
/// How long a pane's border flashes after the terminal bell.
const BELL_FLASH: Duration = Duration::from_millis(200);

/// One pane being drawn this frame.
struct PaneView {
    id: PaneId,
    /// The whole card, header included, where it is drawn (mid-animation, maybe not yet
    /// where the layout puts it).
    card: Rect,
    /// Where the layout puts the card: terminals are sized and hit-tested against this.
    target: Rect,
    focused: bool,
    /// How focused the pane looks, 0 to 1 (fades when focus moves).
    focus: f32,
    /// The tab shows more than one pane (focus is highlighted, the rest dimmed).
    multi: bool,
    /// Being dragged by its header: drawn on top, lifted off the page.
    lifted: bool,
    /// Opacity (below 1 while fading in).
    alpha: f32,
    /// Drawn over the other panes (shrinking back from a zoom).
    on_top: bool,
    /// The content scales with the card rather than being clipped by it (zooming).
    scale_content: bool,
}

/// The lists `panes_area` fills each frame. They stay on `App` between frames, like
/// `term_rects`, so a frame reuses their allocations instead of making new ones.
#[derive(Default)]
pub(in crate::app) struct DrawBuffers {
    /// Where the layout puts each pane of the shown tab.
    rects: Vec<(PaneId, Rect)>,
    dividers: Vec<Divider>,
    /// The panes to draw, in paint order, apart from those in `lifted`.
    views: Vec<PaneView>,
    /// The dragged card, drawn over the others.
    lifted: Vec<PaneView>,
}

impl DrawBuffers {
    fn clear(&mut self) {
        self.rects.clear();
        self.dividers.clear();
        self.views.clear();
        self.lifted.clear();
    }
}

/// How long until the cursor's next blink, `now` seconds into the app's run.
fn next_blink(now: f64) -> Duration {
    let (ms, interval) = (now * 1000.0, BLINK_INTERVAL.as_millis() as f64);
    Duration::from_millis((interval - ms % interval) as u64 + 1)
}

impl App {
    pub(in crate::app) fn panes_area(&mut self, ui: &mut Ui, area: Rect, theme: &Theme) {
        let mut bufs = std::mem::take(&mut self.draw_bufs);
        self.panes_area_with(ui, area, theme, &mut bufs);
        self.draw_bufs = bufs;
    }

    /// [`Self::panes_area`], with the frame's lists in `bufs`.
    fn panes_area_with(&mut self, ui: &mut Ui, area: Rect, theme: &Theme, bufs: &mut DrawBuffers) {
        self.term_rects.clear();
        self.card_rects.clear();
        self.focused_card = None;
        let ctx = ui.ctx().clone();
        let headers = self.cfg.pane_headers.then(|| self.header_height());
        let ppp = ctx.pixels_per_point();
        let zoom = self.zoom_scale(area, ppp, bufs);
        self.set_zoom_font(&ctx, zoom);
        let Some(tab_uid) = self.cur_tab().map(|t| t.uid) else { return };
        // A newly shown tab takes over from the last one (`tab-animation`).
        let shown = Shown {
            uid: tab_uid,
            pos: if self.home_active { -1 } else { self.active as isize },
            area,
            origin: self.drop_origin(tab_uid, area),
            vertical: self.cfg.sidebar(),
            style: self.cfg.tab_animation,
        };
        let sheet = self.switch.begin(ui, &shown);
        bufs.clear();
        let DrawBuffers { rects, dividers, views, lifted } = bufs;
        match self.cur_tab() {
            Some(tab) if tab.zoomed => rects.push((tab.focused, area)),
            Some(tab) => tab.root.layout(area, PANE_GAP, rects, dividers),
            None => return,
        }
        let DragFrame { goals, held, mini, dropped } = self.track_pane_drag(&ctx, area, rects);
        let dragging = held.map(|(id, _)| id);
        if dragging.is_some() {
            ctx.set_cursor_icon(CursorIcon::Grabbing);
        }
        // Carried over from another tab: drawn over this one's panes, not laid out with them.
        let visitor = held.filter(|(id, _)| !rects.iter().any(|(r, _)| r == id));
        let sticky = self.sticky;
        let Some(tab) = self.cur_tab_mut() else { return };
        let focused = tab.focused;
        let multi = rects.len() > 1;
        for p in tab.motion.frame(&ctx, rects, &goals, held) {
            views.push(PaneView {
                id: p.id,
                card: p.card,
                target: p.target,
                focused: p.id == focused,
                focus: tab.motion.focus_amount(&ctx, p.id, p.id == focused),
                multi,
                lifted: Some(p.id) == dragging,
                alpha: p.alpha,
                on_top: p.on_top,
                scale_content: p.scale_content || (mini && Some(p.id) == dragging),
            });
        }
        // The dragged card goes over the others (and a pane shrinking out of a zoom over
        // the panes fading back in).
        views.sort_by_key(|v| (v.lifted, v.on_top));
        // Drawn after the tab's sheet, so a tab switch doesn't carry them off with the old
        // tab: the sticky pane (it glides to its place in the new tab), then the dragged card.
        let pinned = sticky.and_then(|s| views.iter().position(|v| v.id == s)).map(|i| views.remove(i));
        if let Some(i) = views.iter().position(|v| v.lifted) {
            lifted.extend(views.drain(i..));
        }
        // Closed panes fade out underneath the panes growing into their space.
        let now = ctx.input(|i| i.time);
        for g in tab.motion.ghosts() {
            paint_ghost(ui.painter(), g, now, theme, headers);
        }
        self.pending.extend(dropped);
        for (id, t) in self.panes.terminals() {
            t.set_visible(rects.iter().any(|(r, _)| *r == id) || visitor.is_some_and(|(v, _)| v == id));
        }
        self.drag_dividers(ui, tab_uid, dividers);

        let blink_on = {
            let ms = ui.input(|i| i.time * 1000.0) as u128;
            (ms / BLINK_INTERVAL.as_millis()).is_multiple_of(2)
        };
        // Where the connect screen's card lands (`animation/handoff.rs`): the layout's rect,
        // not where the card is drawn mid-animation.
        self.focused_card =
            views.iter().find(|v| v.focused).map(|v| FocusedCard { rect: v.target, ring: if v.multi { v.focus } else { 0.0 }, label: None });
        // Whether anything drawn blinks, and so needs the next frame on time.
        let mut blinks = false;
        for pane in views.iter() {
            blinks |= self.draw_pane(ui, pane, theme, blink_on);
        }
        self.switch.end(ui, sheet);
        match pinned {
            Some(mut pane) => {
                let (card, gliding) = self.sticky_glide.frame(&ctx, tab_uid, pane.card);
                pane.card = card;
                pane.scale_content |= gliding;
                blinks |= self.draw_pane(ui, &pane, theme, blink_on);
            }
            None => self.sticky_glide.hidden(),
        }
        for pane in lifted.iter() {
            blinks |= self.draw_pane(ui, pane, theme, blink_on);
        }
        if let Some((id, card)) = visitor {
            self.visiting_pane(ui, id, card, mini, theme, blink_on);
        }

        // Keep blinking cursors blinking. Nothing blinks in an idle or unfocused window, and
        // it is left asleep rather than woken every interval to draw the same frame.
        if blinks {
            ui.ctx().request_repaint_after(next_blink(ui.input(|i| i.time)));
        }
    }

    /// One pane of the shown tab: a terminal, a file explorer, a Changes or an Agents pane.
    /// Returns whether it shows something blinking.
    fn draw_pane(&mut self, ui: &mut Ui, pane: &PaneView, theme: &Theme, blink_on: bool) -> bool {
        let Some(kind) = self.pane_kind(pane.id) else { return false };
        let blinks = self.draw_content(ui, kind, pane, theme, blink_on);
        self.card_rects.push((pane.id, pane.target));
        blinks
    }

    /// The card of pane `pane`, a `kind` pane: its header and what it shows. Returns whether
    /// it shows something blinking (a terminal's cursor, the search field's caret).
    fn draw_content(&mut self, ui: &mut Ui, kind: PaneKind, pane: &PaneView, theme: &Theme, blink_on: bool) -> bool {
        match kind {
            PaneKind::Terminal => return self.terminal_pane(ui, pane, theme, blink_on),
            PaneKind::Explorer => self.explorer_pane(ui, pane, theme),
            PaneKind::Diff => self.diff_pane(ui, pane, theme),
            PaneKind::Agents => self.agents_pane(ui, pane, theme),
        }
        false
    }

    /// A pane dragged here from another tab, held at `card`. Its terminal keeps the size of
    /// its slot back home, and it takes no part in this tab's hit-testing.
    fn visiting_pane(&mut self, ui: &mut Ui, id: PaneId, card: Rect, mini: bool, theme: &Theme, blink_on: bool) {
        let Some(slot) = self.input.pane_drag.as_ref().map(|d| d.slot) else { return };
        let pane =
            PaneView { id, card, target: slot, focused: false, focus: 1.0, multi: true, lifted: true, alpha: 1.0, on_top: true, scale_content: mini };
        let Some(kind) = self.pane_kind(id) else { return };
        self.draw_content(ui, kind, &pane, theme, blink_on);
        // It takes no part in this tab's mouse hit-testing.
        self.term_rects.retain(|(p, _)| *p != id);
    }

    /// Where tab `uid` is in the strip (its tab, or the Files label), brought onto the
    /// nearest edge of the pane area: where the ink drops when it is shown.
    fn drop_origin(&self, uid: u64, area: Rect) -> Pos2 {
        let at = if self.home_active { self.chrome.strip.files_rect } else { self.chrome.strip.tab_rects.get(&uid).map(|d| d.rect) };
        at.map_or(area.center_top(), |r| area.clamp(r.center()))
    }

    /// While the shown tab is zoomed: its zoomed pane and how much bigger its terminal area
    /// is than in its slot, so its text scales up to keep the same grid.
    fn zoom_scale(&self, area: Rect, ppp: f32, bufs: &mut DrawBuffers) -> Option<(PaneId, f32)> {
        let tab = self.cur_tab().filter(|t| t.zoomed)?;
        bufs.clear();
        tab.root.layout(area, PANE_GAP, &mut bufs.rects, &mut bufs.dividers);
        let slot = bufs.rects.iter().find(|(id, _)| *id == tab.focused)?.1;
        let (full, slot) = (self.term_rect_for(area, ppp), self.term_rect_for(slot, ppp));
        if !slot.is_positive() {
            return None;
        }
        Some((tab.focused, (full.width() / slot.width()).min(full.height() / slot.height()).max(1.0)))
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
        let painter = &pane_painter(ui, pane);
        let ring = if pane.multi { pane.focus } else { 0.0 };
        if pane.lifted {
            let shadow = Shadow { offset: [0, 10], blur: 36, spread: 2, color: Color32::from_black_alpha(130) };
            painter.add(shadow.as_shape(pane.card, PANE_RADIUS));
        }
        paint::card(painter, pane.card, PANE_RADIUS, theme.card, ring, accent);
        if !self.cfg.pane_headers {
            return pane.card.top() + self.cfg.window_padding * 0.5;
        }
        let header = Rect::from_min_size(pane.card.min, Vec2::new(pane.card.width(), self.header_height()));
        self.pane_header(ui, header, pane, theme);
        header.bottom()
    }

    /// Unfocused splits are dimmed (`unfocused-split-opacity`), fading in and out with focus.
    fn dim_unfocused(&self, painter: &Painter, pane: &PaneView) {
        let dim = (1.0 - self.cfg.unfocused_split_opacity) * (1.0 - pane.focus);
        if pane.multi && dim > 0.0 {
            painter.rect_filled(pane.card, PANE_RADIUS, darken(self.cfg.background, 0.5).gamma_multiply(dim));
        }
    }

    /// A terminal pane. Returns whether it shows something blinking: its cursor, or the
    /// search field's caret.
    fn terminal_pane(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme, blink_on: bool) -> bool {
        let ctx = ui.ctx().clone();
        let painter = &pane_painter(ui, pane);
        let id = pane.id;
        let bell = self.panes.terminal(id).is_some_and(|t| t.bell_at.is_some_and(|at| at.elapsed() < BELL_FLASH));
        let accent = bell.then_some(self.cfg.palette[3]);
        let content_top = self.pane_frame(ui, pane, theme, accent);

        // Sized for where the layout puts the pane, so an animating card never resizes the PTY.
        let term_rect = self.term_rect_for(pane.target, ctx.pixels_per_point());
        // Where the content is drawn mid-zoom, scaled to fit the card.
        let card_term = self.term_rect_for(pane.card, ctx.pixels_per_point());
        let m = self.pane_metrics(id);
        let cell_px = cell_px(&ctx, m);
        let cols = ((term_rect.width() / m.width).floor() as usize).max(2);
        let lines = ((term_rect.height() / m.height).floor() as usize).max(1);
        let Some(t) = self.panes.terminal_mut(id) else { return false };
        t.resize(GridSize { cols, lines }, cell_px);
        let typing = t.last_input.elapsed() < BLINK_INTERVAL;

        // One lock for everything read from the terminal here.
        let (search, offset, selection, blinking) = {
            let mut term = t.term.lock();
            // Keep search results in sync with the terminal contents.
            let search = self.input.search.as_mut().filter(|s| s.pane == id).map(|s| {
                if let Some(p) = s.refresh(&term, t.generation) {
                    term.scroll_to_point(p);
                }
                &*s
            });
            // Read after the search refresh, which may scroll to a match.
            let selection = term.selection.as_ref().and_then(|s| s.to_range(&term));
            (search, term.grid().display_offset(), selection, term.cursor_style().blinking)
        };
        let link = self.input.hover_link.as_ref().filter(|(pid, _)| *pid == id).map(|(_, l)| &l.range);
        let window_focused = ctx.input(|i| i.focused);
        // What blinks: the focused pane's cursor, if its style says so, and the search field's
        // caret, both only in a focused window. Whatever doesn't blink is shown steadily, as
        // no frame is scheduled to bring it back.
        let cursor_blinks = cursor_blinks(pane.focused, window_focused, blinking);
        let caret_blinks = search.is_some() && window_focused;
        let opts = PaintOptions {
            focused: pane.focused && window_focused,
            cursor_visible: !cursor_blinks || typing || blink_on,
            bg_alpha: self.cfg.background_opacity,
            matches: search.map(|s| s.matches.as_slice()).unwrap_or_default(),
            current_match: search.and_then(|s| s.current()),
            link,
        };
        let key = FrameKey {
            generation: t.generation,
            display_offset: offset,
            selection,
            rect: term_rect,
            metrics: m,
            focused: opts.focused,
            cursor_visible: opts.cursor_visible,
            bg_alpha: opts.bg_alpha,
            search: search.map(|s| (s.results_generation(), s.index)),
            link: link.cloned(),
            glyph_epoch: self.glyphs.epoch(),
        };
        let (glyphs, cfg) = (&mut self.glyphs, &self.cfg);
        let shapes = t.frame_shapes(key, |snap| render::terminal_shapes(painter, term_rect, snap, glyphs, cfg, m, opts));
        // Mid-animation the content rides along with the card's corner, clipped to the card;
        // while zooming it scales with the card too.
        let clip = painter.with_clip_rect(pane.card.shrink(1.0));
        let shift = pane.card.min - pane.target.min;
        if pane.scale_content && pane.card != pane.target {
            let at = card_term;
            let scale = (at.width() / term_rect.width()).min(at.height() / term_rect.height()).max(0.0);
            let t = TSTransform::new(at.min.to_vec2() - term_rect.min.to_vec2() * scale, scale);
            clip.extend(shapes.iter().cloned().map(|mut s| {
                s.transform(t);
                s
            }));
        } else if shift == Vec2::ZERO {
            clip.extend(shapes.iter().cloned());
        } else {
            clip.extend(shapes.iter().cloned().map(|mut s| {
                s.translate(shift);
                s
            }));
        }
        if let Some(s) = search {
            search_bar(painter, pane.card, content_top, s, theme, !caret_blinks || blink_on);
        }
        self.dim_unfocused(painter, pane);

        if offset > 0 {
            scrollback_badge(painter, pane.card, content_top, offset, theme);
        }
        self.term_rects.push((id, term_rect));
        cursor_blinks || caret_blinks
    }

    /// What the explorer, Changes and Agents panes share: the card and header, a press
    /// anywhere on the body focusing the pane, `paint` drawing the body, and the dimming of
    /// an unfocused split over it. `paint` is given a painter clipped to the card and the
    /// body's rect: inside the card's padding like a terminal's grid, or with `edge_to_edge`
    /// the whole card below the header. It also hands on what the pane asked for while
    /// painting, straight into `App`'s list of requests.
    fn pane_body(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme, edge_to_edge: bool, paint: impl FnOnce(&mut Self, &Painter, Rect)) {
        let top = self.pane_frame(ui, pane, theme, None);
        let painter = pane_painter(ui, pane);
        // An edge-to-edge body stays inside the card's border, but is pressed up to it.
        let (pressable, content) = if edge_to_edge {
            let below = Rect::from_min_max(Pos2::new(pane.card.left(), top), pane.card.max);
            (below, below.shrink2(Vec2::new(1.0, 0.0)))
        } else {
            let padded = self.term_rect_for(pane.card, ui.ctx().pixels_per_point());
            (padded, padded)
        };
        if pressed_in(ui, pressable) {
            self.pending.push(Action::FocusPane(pane.id));
        }
        paint(self, &painter.with_clip_rect(pane.card.shrink(1.0)), content);
        self.dim_unfocused(&painter, pane);
    }

    /// A file-explorer pane: the usual card and header, and the listing. It isn't added to
    /// `term_rects`, so terminal mouse handling ignores it.
    fn explorer_pane(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme) {
        let id = pane.id;
        self.pane_body(ui, pane, theme, false, |app, painter, content| {
            if let Some(b) = app.panes.explorer_mut(id) {
                b.paint(ui, painter, content, id, theme);
                app.pane_requests.extend(b.requests.drain(..).map(|r| (id, PaneRequest::Explorer(r))));
            }
        });
    }

    /// A Changes (git diff) pane: the usual card and header, then the pane's own toolbar,
    /// file list and diff, edge to edge of the card below the header.
    fn diff_pane(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme) {
        let (id, focused) = (pane.id, pane.focused);
        let size = self.render_font(id);
        self.pane_body(ui, pane, theme, true, |app, painter, content| {
            if let Some(d) = app.panes.diff_mut(id) {
                d.paint(ui, painter, content, id, focused, PANE_RADIUS - 1, theme, size);
                app.pane_requests.extend(d.requests.drain(..).map(|r| (id, PaneRequest::Diff(r))));
            }
        });
    }

    /// An Agents pane: the usual card and header, then the pane's title, filter and agent
    /// cards, edge to edge of the card below the header.
    fn agents_pane(&mut self, ui: &Ui, pane: &PaneView, theme: &Theme) {
        let (id, focused) = (pane.id, pane.focused);
        self.pane_body(ui, pane, theme, true, |app, painter, content| {
            let viewing = app.viewing(id);
            if let Some(a) = app.panes.agents_mut(id) {
                let shown_tab = (!app.home_active).then_some(app.active);
                let args = crate::agents::PaintArgs {
                    pane: id,
                    focused,
                    c: theme,
                    agents: app.agent_tracker.agents(),
                    notes: &app.ssh.agent_notes,
                    shown_tab,
                    viewing: &viewing,
                };
                a.paint(ui, painter, content, args);
                app.pane_requests.extend(a.requests.drain(..).map(|r| (id, PaneRequest::Agents(r))));
            }
        });
    }

    /// Process glyph, title, and (on the focused or hovered card only, keeping the other
    /// headers clean) close / zoom / split buttons. Clicking the header focuses the pane.
    fn pane_header(&mut self, ui: &Ui, header: Rect, pane: &PaneView, theme: &Theme) {
        // Clipped to the card, which may still be growing out of a split.
        let painter = &pane_painter(ui, pane).with_clip_rect(pane.card);
        let id = pane.id;
        let zoomed = self.cur_tab().is_some_and(|t| t.zoomed);
        let show_buttons = !pane.lifted && (pane.focused || ui.rect_contains_pointer(pane.card));

        // The header is the handle for dragging the pane: onto another pane, or to the tabs
        // (see `pane_drag`). It is registered before the buttons, so they stay on top and
        // keep their clicks. A tab's only pane can still be dragged to another tab.
        let handle = ui.interact(header, Id::new(("pane-drag", id)), Sense::click_and_drag());
        if pane.multi || (self.tabs.len() > 1 && !self.home_active) {
            if handle.hovered() && !handle.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            }
            if handle.drag_started_by(PointerButton::Primary)
                && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(handle.interact_pointer_pos())
            {
                self.start_pane_drag(id, at, pane.target);
            }
        }

        let cy = header.center().y + 1.0;
        let title_color = theme.text.gamma_multiply(0.85).lerp_to_gamma(theme.text_bright, pane.focus);

        // Buttons, laid out right to left: 28px buttons like the tab bar's +, with smaller (~12px)
        // glyphs so the header stays quiet. Mobile mode makes them finger-sized.
        let (size, step, inset) = if self.cfg.sidebar() { (36.0, 38.0, 7.0) } else { (DESKTOP_BUTTON, 30.0, DESKTOP_INSET) };
        let mut bx = header.right() - if self.cfg.sidebar() { 26.0 } else { 22.0 };
        let pinned = self.is_sticky(id);
        // Holding Shift turns the × into a pin: the pane follows the user to every tab. On
        // the pinned pane that slot is the pin (click to unpin, back to the ×), and Shift
        // brings the × back to close it. On Android, Ctrl armed with the keyboard button (no
        // Shift to hold there) does the same for the focused pane, and the tap uses it up.
        let armed = pane.focused && self.chrome.keyboard_button.ctrl_armed;
        let shift = ui.input(|i| i.modifiers.shift) || armed;
        // The pinned pane's pin shows even when its other buttons are hidden.
        let shown = if show_buttons {
            4
        } else if pinned {
            1
        } else {
            0
        };
        if shown > 0 {
            let close: (&str, Action, Action, &IconFn<'_>) = match (pinned, shift) {
                (true, false) => ("Unpin from other tabs  (Shift: close)", Action::ToggleSticky(0), Action::ToggleSticky(id), &icons::icon_pin),
                (false, true) => ("Pin to every tab", Action::ToggleSticky(0), Action::ToggleSticky(id), &icons::icon_pin),
                _ => ("Close  (Shift: pin to every tab)", Action::ClosePane(0), Action::ClosePane(id), &icons::icon_close),
            };
            // (label, the shortcut's action for the focused pane, this button's action, icon)
            let buttons: [(&str, Action, Action, &IconFn<'_>); 4] = [
                close,
                ("Zoom", Action::ToggleZoom, Action::ToggleZoomPane(id), &|p, r, c| icons::icon_zoom(p, r, c, zoomed)),
                ("Split down", Action::Split(Axis::Vertical), Action::SplitPane(id, Axis::Vertical), &|p, r, c| {
                    icons::icon_split(p, r, c, Axis::Vertical)
                }),
                ("Split right", Action::Split(Axis::Horizontal), Action::SplitPane(id, Axis::Horizontal), &|p, r, c| {
                    icons::icon_split(p, r, c, Axis::Horizontal)
                }),
            ];
            for (i, (label, shortcut, action, draw)) in buttons.into_iter().enumerate().take(shown) {
                let tip = match keymap::shortcut_label(&self.bindings, shortcut) {
                    Some(keys) => format!("{label}  ({keys})"),
                    None => label.to_string(),
                };
                let tip = tip.as_str();
                let r = Rect::from_center_size(Pos2::new(bx, cy), Vec2::splat(size));
                bx -= step;
                // Keyed by slot, so the × turning into a pin keeps its hover.
                // The pin stands out a little: it's a state, not just a button.
                let normal = if i == 0 && pinned && !shift { theme.text_bright.gamma_multiply(0.85) } else { theme.text_dim };
                if IconButton::new(r, Id::new(("pane-btn", id, i)), normal, theme.text_bright)
                    .icon_rect(r.shrink(inset))
                    .hover(HoverShape::Rounded(8), HOVER)
                    .tooltip(tip)
                    .show(ui, painter, draw)
                {
                    if i == 0 && armed {
                        self.chrome.keyboard_button.ctrl_armed = false;
                    }
                    self.pending.push(action);
                }
            }
        }

        // The mic button, for a pane running ssh: on hover like the others, but always while
        // the mic is shared, so it's never on without showing it.
        if let Some(mic) = self.mic_state(id).filter(|&m| show_buttons || m != MicState::Off) {
            let host = self.mic_host(id).unwrap_or_default();
            let label = match mic {
                MicState::Off => format!("Share microphone with {host}"),
                MicState::Shared => format!("Stop sharing microphone with {host}"),
                MicState::Live => format!("Stop sharing microphone with {host} (recording)"),
            };
            let tip = match keymap::shortcut_label(&self.bindings, Action::ToggleMic(0)) {
                Some(keys) if pane.focused => format!("{label}  ({keys})"),
                _ => label,
            };
            let (normal, hover) = match mic {
                MicState::Live => (ERROR, ERROR),
                MicState::Shared => (theme.text_bright, theme.text_bright),
                MicState::Off => (theme.text_dim, theme.text_bright),
            };
            let r = Rect::from_center_size(Pos2::new(bx, cy), Vec2::splat(size));
            bx -= step;
            if IconButton::new(r, Id::new(("pane-mic", id)), normal, hover)
                .icon_rect(r.shrink(inset))
                .hover(HoverShape::Rounded(8), HOVER)
                .tooltip(&tip)
                .show(ui, painter, |p: &Painter, r: Rect, c: Color32| icons::icon_mic(p, r, c, mic != MicState::Off))
            {
                self.pending.push(Action::ToggleMic(id));
            }
        }

        let font = if self.cfg.sidebar() { 15.0 } else { 13.5 };
        // Borrowed from the pane, so read here, once the buttons are done changing things.
        let (title, process) = self.pane_label(id);
        header_label(painter, header, process, &title, title_color, bx - 4.0, font);
        // The connect handoff draws the same label on its card as it lands here: a copy it
        // keeps, made only while it plays.
        if self.handoff.is_some() && self.focused_card.as_ref().is_some_and(|f| pane.focused && f.rect == pane.target) {
            let label = HeaderLabel {
                process: process.to_string(),
                title: title.into_owned(),
                color: title_color,
                right_inset: header.right() - (bx - 4.0),
                font,
            };
            if let Some(f) = self.focused_card.as_mut() {
                f.label = Some(label);
            }
        }
        if pressed_in(ui, header) {
            self.pending.push(Action::FocusPane(id));
        }
    }
}

/// Whether a pane's cursor blinks: only the focused pane's, in a focused window, when the
/// terminal's cursor style blinks. Any other cursor is drawn steadily.
fn cursor_blinks(pane_focused: bool, window_focused: bool, style_blinks: bool) -> bool {
    pane_focused && window_focused && style_blinks
}

/// Process glyph and title at the left of a pane header, the title fading out before `right`.
pub(in crate::app) fn header_label(painter: &Painter, header: Rect, process: &str, title: &str, color: Color32, right: f32, font: f32) {
    let (mark, title) = icons::title_mark(process, title);
    icons::header_glyph(painter, header_glyph_rect(header), process, mark, color);
    header_title(painter, header, title, color, right, font);
}

/// Where a pane header's process glyph goes (the connect card puts the dog there).
pub(in crate::app) fn header_glyph_rect(header: Rect) -> Rect {
    Rect::from_center_size(Pos2::new(header.left() + 20.0, header.center().y + 1.0), Vec2::splat(13.0))
}

/// A pane header's title, after the glyph, faded out before `right`.
pub(in crate::app) fn header_title(painter: &Painter, header: Rect, title: &str, color: Color32, right: f32, font: f32) {
    let glyph = header_glyph_rect(header);
    let text_left = glyph.right() + 8.0;
    if right > text_left {
        paint::faded_text(painter, Pos2::new(text_left, glyph.center().y), title, paint::bold(font), color, right - text_left, 24.0);
    }
}

/// A closed pane on its way out: its last frame, collapsing toward its outer edge and fading.
/// `header` is the pane-header height, if headers are shown.
/// The painter for one pane, faded while it fades in.
fn pane_painter(ui: &Ui, pane: &PaneView) -> Painter {
    let mut p = ui.painter().clone();
    p.multiply_opacity(pane.alpha);
    p
}

fn paint_ghost(painter: &Painter, g: &Ghost, now: f64, theme: &Theme, header: Option<f32>) {
    let (card, shift, alpha) = g.at(now);
    let mut p = painter.clone();
    p.multiply_opacity(alpha);
    paint::card(&p, card, PANE_RADIUS, theme.card, 0.0, None);
    let clip = p.with_clip_rect(card.shrink(1.0));
    if let Some(h) = header {
        let header = Rect::from_min_size(card.min, Vec2::new(card.width(), h));
        header_label(&clip, header, &g.process, &g.title, theme.text.gamma_multiply(0.85), header.right() - 20.0, 13.5);
    }
    clip.extend(g.shapes.iter().cloned().map(|mut s| {
        s.translate(shift);
        s
    }));
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
    painter.rect_stroke(r, 8, Stroke::new(1.0_f32, Color32::from_white_alpha(40)), StrokeKind::Inside);

    // Magnifier icon.
    let c = Pos2::new(r.left() + 16.0, r.center().y - 1.0);
    painter.circle_stroke(c, 4.5, Stroke::new(1.4_f32, theme.text_dim));
    painter.line_segment([c + Vec2::new(3.2, 3.2), c + Vec2::new(6.5, 6.5)], Stroke::new(1.4_f32, theme.text_dim));

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
        clip.line_segment([Pos2::new(caret_x, r.center().y - 7.0), Pos2::new(caret_x, r.center().y + 7.0)], Stroke::new(1.2_f32, theme.text));
    }
}

#[cfg(test)]
mod tests {
    use egui::{RawInput, ViewportId};

    use super::*;
    use crate::config::Config;

    fn app(ctx: &egui::Context, cfg: Config) -> App {
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        App::new(&cc, cfg, crate::cli::Args { no_restore: true, ..Default::default() }, None, None, None)
    }

    /// One frame of `app`, `time` seconds in, in a window that has the keyboard focus or not.
    /// Returns how long the app is content to sleep before its next frame.
    fn frame(app: &mut App, ctx: &egui::Context, time: f64, focused: bool) -> Duration {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let input = RawInput { screen_rect: Some(screen), time: Some(time), focused, ..Default::default() };
        let out = ctx.run(input, |ctx| {
            let mut frame = eframe::Frame::_new_kittest();
            eframe::App::update(app, ctx, &mut frame);
        });
        out.viewport_output[&ViewportId::ROOT].repaint_delay
    }

    /// The longest the app would sleep over two seconds of frames, once the animations of
    /// its first seconds are over.
    fn longest_sleep(app: &mut App, ctx: &egui::Context, focused: bool) -> Duration {
        let frames = |from: f64| (0..40).map(move |i| from + f64::from(i) * 0.05);
        for time in frames(0.0) {
            frame(app, ctx, time, focused);
        }
        frames(2.0).map(|time| frame(app, ctx, time, focused)).max().unwrap()
    }

    /// A frame at each blink is scheduled only while something blinks: the focused
    /// terminal's blinking cursor, in a focused window. Otherwise the window sleeps longer.
    #[test]
    fn blink_frames_are_scheduled_only_for_a_blinking_cursor_in_a_focused_window() {
        let a_blink = BLINK_INTERVAL + Duration::from_millis(1);

        let ctx = egui::Context::default();
        let mut blinking = app(&ctx, Config::default());
        assert!(longest_sleep(&mut blinking, &ctx, true) <= a_blink, "a blinking cursor needs a frame at every blink");
        assert!(longest_sleep(&mut blinking, &ctx, false) > a_blink, "nothing blinks in an unfocused window");

        let ctx = egui::Context::default();
        let mut steady = app(&ctx, Config { cursor_blink: false, ..Config::default() });
        assert!(longest_sleep(&mut steady, &ctx, true) > a_blink, "a steady cursor needs no frames");
    }

    #[test]
    fn only_the_focused_cursor_of_a_focused_window_blinks() {
        assert!(cursor_blinks(true, true, true));
        assert!(!cursor_blinks(false, true, true), "an unfocused pane's cursor is steady");
        assert!(!cursor_blinks(true, false, true), "so is every cursor of an unfocused window");
        assert!(!cursor_blinks(true, true, false), "and one whose style doesn't blink");
    }

    #[test]
    fn the_next_blink_is_at_the_next_multiple_of_the_interval() {
        assert_eq!(next_blink(0.0), Duration::from_millis(601));
        assert_eq!(next_blink(0.5), Duration::from_millis(101));
        assert_eq!(next_blink(1.25), Duration::from_millis(551));
    }

    /// The frame's lists go back to `App` when it is drawn, and the next frame fills the
    /// same allocations.
    #[test]
    fn frames_reuse_the_pane_lists() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx, Config::default());
        frame(&mut app, &ctx, 0.0, true);
        let lists = |app: &App| (app.draw_bufs.rects.as_ptr(), app.draw_bufs.views.as_ptr());
        let first = lists(&app);
        assert_eq!(app.draw_bufs.rects.len(), 1, "the one pane's rect, kept after the frame");
        assert_eq!(app.draw_bufs.views.len(), 1);
        for i in 1..10 {
            frame(&mut app, &ctx, f64::from(i) * 0.05, true);
            assert_eq!(lists(&app), first, "frame {i} allocated new lists");
        }
    }
}
