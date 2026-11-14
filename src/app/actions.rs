//! Everything the user can ask for (keyboard shortcuts, buttons), and carrying it out.

use alacritty_terminal::grid::Scroll;
use egui::{Key, ViewportCommand};

use crate::layout::{self, Axis, Direction, PaneId};
use crate::search::Search;

use super::App;
use super::zoom::EXPLORER_ZOOM_STEP;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Action {
    NewTab,
    NewWindow,
    /// Close a pane; `ClosePane(0)` means the focused one.
    ClosePane(PaneId),
    CloseTab(usize),
    Split(Axis),
    SplitPane(PaneId, Axis),
    Focus(Direction),
    FocusPane(PaneId),
    ResizeSplit(Direction),
    ToggleZoom,
    ToggleZoomPane(PaneId),
    Equalize,
    NextTab,
    PrevTab,
    GotoTab(usize),
    Copy,
    FontBigger,
    FontSmaller,
    FontReset,
    ReloadConfig,
    Quit,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    ClearScreen,
    ToggleSearch,
    /// Open the Files tab, switch to it, or (from it) back to the previous tab.
    ToggleExplorer,
    /// Split a file explorer off the focused pane, showing its directory.
    ExploreHere,
    CloseExplorer,
}

/// A keyboard shortcut: (ctrl, shift, alt, key, action).
pub(super) type Binding = (bool, bool, bool, Key, Action);

/// Keyboard shortcuts, following Ghostty's Linux defaults where they exist.
pub(super) fn keybindings() -> Vec<Binding> {
    use Action::*;
    let mut b = vec![
        (true, true, false, Key::T, NewTab),
        (true, true, false, Key::N, NewWindow),
        (true, true, false, Key::W, ClosePane(0)),
        (true, true, false, Key::Q, Quit),
        (true, true, false, Key::O, Split(Axis::Horizontal)),
        (true, true, false, Key::E, Split(Axis::Vertical)),
        (true, true, false, Key::Enter, ToggleZoom),
        (true, false, true, Key::ArrowLeft, Focus(Direction::Left)),
        (true, false, true, Key::ArrowRight, Focus(Direction::Right)),
        (true, false, true, Key::ArrowUp, Focus(Direction::Up)),
        (true, false, true, Key::ArrowDown, Focus(Direction::Down)),
        (true, true, true, Key::ArrowLeft, ResizeSplit(Direction::Left)),
        (true, true, true, Key::ArrowRight, ResizeSplit(Direction::Right)),
        (true, true, true, Key::ArrowUp, ResizeSplit(Direction::Up)),
        (true, true, true, Key::ArrowDown, ResizeSplit(Direction::Down)),
        (true, true, true, Key::Equals, Equalize),
        (true, true, true, Key::Plus, Equalize),
        (true, false, false, Key::Tab, NextTab),
        (true, true, false, Key::Tab, PrevTab),
        (true, false, false, Key::PageDown, NextTab),
        (true, false, false, Key::PageUp, PrevTab),
        (true, true, false, Key::ArrowRight, NextTab),
        (true, true, false, Key::ArrowLeft, PrevTab),
        (true, false, false, Key::Equals, FontBigger),
        (true, false, false, Key::Plus, FontBigger),
        (true, true, false, Key::Plus, FontBigger),
        (true, true, false, Key::Equals, FontBigger),
        (true, false, false, Key::Minus, FontSmaller),
        (true, false, false, Key::Num0, FontReset),
        (true, true, false, Key::Comma, ReloadConfig),
        (true, true, false, Key::K, ClearScreen),
        (true, true, false, Key::F, ToggleSearch),
        (true, false, false, Key::E, ExploreHere),
        (false, true, false, Key::PageUp, ScrollPageUp),
        (false, true, false, Key::PageDown, ScrollPageDown),
        (false, true, false, Key::Home, ScrollTop),
        (false, true, false, Key::End, ScrollBottom),
    ];
    let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    for (i, k) in digits.into_iter().enumerate() {
        b.push((false, false, true, k, GotoTab(i)));
    }
    b
}

impl App {
    pub(super) fn apply(&mut self, ctx: &egui::Context, action: Action) {
        let focused = self.focused_pane();
        match action {
            Action::NewTab => {
                let cwd = self.focused_cwd();
                self.new_tab(ctx, cwd);
            }
            Action::NewWindow => self.new_window(),
            Action::ClosePane(0) => {
                if let Some(id) = focused {
                    self.close_pane(ctx, id)
                }
            }
            Action::ClosePane(id) => self.close_pane(ctx, id),
            Action::CloseTab(i) => self.close_tab(i),
            Action::Split(axis) => {
                if let Some(id) = focused {
                    self.split(ctx, id, axis)
                }
            }
            Action::SplitPane(id, axis) => self.split(ctx, id, axis),
            Action::Focus(dir) => self.focus_direction(dir),
            Action::FocusPane(id) => {
                if let Some(tab) = self.tab_with(id) {
                    tab.focused = id;
                }
            }
            Action::ResizeSplit(dir) => {
                if let (Some(id), Some(tab)) = (focused, self.cur_tab_mut()) {
                    tab.root.resize(id, dir, 0.05);
                }
            }
            Action::ToggleZoom => self.toggle_zoom(None),
            Action::ToggleZoomPane(id) => self.toggle_zoom(Some(id)),
            Action::Equalize => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.root.equalize();
                }
            }
            Action::NextTab => self.cycle_tab(true),
            Action::PrevTab => self.cycle_tab(false),
            Action::GotoTab(i) => self.goto_tab(i),
            Action::Copy => {
                if let Some(text) = focused.and_then(|id| self.panes.get(&id)).and_then(|t| t.selection_text()) {
                    ctx.copy_text(text);
                }
            }
            Action::FontBigger | Action::FontSmaller | Action::FontReset => self.change_font(ctx, action),
            Action::ReloadConfig => self.reload_config(ctx),
            Action::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Action::ScrollPageUp | Action::ScrollPageDown | Action::ScrollTop | Action::ScrollBottom => {
                self.scroll_focused(action)
            }
            Action::ToggleExplorer => self.toggle_explorer(),
            Action::ExploreHere => {
                if let Some(id) = focused {
                    self.split_explorer(ctx, id, Axis::Horizontal)
                }
            }
            Action::CloseExplorer => self.close_explorer(),
            Action::ToggleSearch => self.toggle_search(),
            Action::ClearScreen => {
                // Ask the shell to redraw on a clean screen: clear + form feed (Ctrl+L).
                if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                    t.input(b"\x0c".to_vec());
                }
            }
        }
    }

    /// Start another nex process in the focused directory, with the same config and app id.
    fn new_window(&self) {
        if let Ok(exe) = std::env::current_exe() {
            let mut cmd = std::process::Command::new(exe);
            cmd.args(&self.new_window_args);
            if let Some(cwd) = self.focused_cwd() {
                cmd.current_dir(cwd);
            }
            let _ = cmd.spawn();
        }
    }

    /// Move focus to the pane next to the focused one, by on-screen position.
    fn focus_direction(&mut self, dir: Direction) {
        let Some(id) = self.focused_pane() else { return };
        let Some(next) = layout::neighbor(&self.card_rects, id, dir) else { return };
        if let Some(tab) = self.cur_tab_mut() {
            tab.focused = next;
        }
    }

    /// Zoom the focused pane (or `pane`, focusing it first) to fill the tab, or unzoom.
    fn toggle_zoom(&mut self, pane: Option<PaneId>) {
        if let Some(tab) = self.cur_tab_mut() {
            if let Some(id) = pane {
                tab.focused = id;
            }
            tab.zoomed = !tab.zoomed && tab.root.panes().len() > 1;
        }
    }

    /// Next/previous tab. The Files tab, when open, sits before the first terminal tab in
    /// the cycle.
    fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        let at_edge = if forward { self.active + 1 == n } else { self.active == 0 };
        if self.explorer_active {
            if n > 0 {
                self.show_tab(if forward { 0 } else { n - 1 });
            }
        } else if at_edge && self.explorer_tab.is_some() {
            self.explorer_active = true;
        } else if n > 0 {
            self.show_tab(if forward { (self.active + 1) % n } else { (self.active + n - 1) % n });
        }
    }

    /// Alt+1..9. Like Ghostty, Alt+9 is "last tab".
    fn goto_tab(&mut self, i: usize) {
        if i == 8 && !self.tabs.is_empty() {
            self.show_tab(self.tabs.len() - 1);
        } else if i < self.tabs.len() {
            self.show_tab(i);
        }
    }

    /// Zoom the focused pane's font in, out, or back to the config size. A file explorer
    /// zooms its whole listing instead.
    fn change_font(&mut self, ctx: &egui::Context, action: Action) {
        let Some(id) = self.focused_pane() else { return };
        if let Some(explorer) = self.explorers.get_mut(&id) {
            match action {
                Action::FontBigger => explorer.zoom_by(EXPLORER_ZOOM_STEP, None),
                Action::FontSmaller => explorer.zoom_by(1.0 / EXPLORER_ZOOM_STEP, None),
                _ => explorer.reset_zoom(),
            }
            return;
        }
        let size = self.pane_font(id);
        let size = match action {
            Action::FontBigger => Some(size + 1.0),
            Action::FontSmaller => Some(size - 1.0),
            _ => None,
        };
        self.set_pane_font(ctx, id, size);
    }

    fn scroll_focused(&mut self, action: Action) {
        let Some(t) = self.focused_pane().and_then(|id| self.panes.get_mut(&id)) else { return };
        t.scroll_to(match action {
            Action::ScrollPageUp => Scroll::PageUp,
            Action::ScrollPageDown => Scroll::PageDown,
            Action::ScrollTop => Scroll::Top,
            _ => Scroll::Bottom,
        });
    }

    /// Open search in the focused terminal. Pressing it again in the same pane closes search;
    /// in another pane, moves it there.
    fn toggle_search(&mut self) {
        // Nothing to search in a file explorer.
        let focused = self.focused_pane().filter(|id| !self.explorers.contains_key(id));
        self.search = match (self.search.take(), focused) {
            (Some(s), Some(id)) if s.pane != id => Some(Search::new(id)),
            (Some(_), _) => None,
            (None, Some(id)) => Some(Search::new(id)),
            (None, None) => None,
        };
    }
}
