//! Everything the user can ask for (keyboard shortcuts, buttons), and carrying it out.


use alacritty_terminal::grid::Scroll;
use egui::{
    Key, ViewportCommand,
};

use crate::config::Config;
use crate::fonts;
use crate::layout::{self, Axis, Direction, PaneId};
use crate::search::Search;

use super::App;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Action {
    NewTab,
    NewWindow,
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
    ToggleFiles,
    CloseFiles,
}

/// Keyboard shortcuts, following Ghostty's Linux defaults where they exist.
/// Each entry is (ctrl, shift, alt, key, action).
pub(super) fn keybindings() -> Vec<(bool, bool, bool, Key, Action)> {
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
            Action::NewWindow => {
                if let Ok(exe) = std::env::current_exe() {
                    let mut cmd = std::process::Command::new(exe);
                    if let Some(cwd) = self.focused_cwd() {
                        cmd.current_dir(cwd);
                    }
                    let _ = cmd.spawn();
                }
            }
            Action::ClosePane(0) => {
                if let Some(id) = focused {
                    self.close_pane(id)
                }
            }
            Action::ClosePane(id) => self.close_pane(id),
            Action::CloseTab(i) => self.close_tab(i),
            Action::Split(axis) => {
                if let Some(id) = focused {
                    self.split(ctx, id, axis)
                }
            }
            Action::SplitPane(id, axis) => self.split(ctx, id, axis),
            Action::Focus(dir) => {
                if let Some(id) = focused {
                    if let Some(next) = layout::neighbor(&self.card_rects, id, dir) {
                        if let Some(tab) = self.cur_tab_mut() {
                            tab.focused = next;
                        }
                    }
                }
            }
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
            Action::ToggleZoom => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.zoomed = !tab.zoomed && tab.root.panes().len() > 1;
                }
            }
            Action::ToggleZoomPane(id) => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.focused = id;
                    tab.zoomed = !tab.zoomed && tab.root.panes().len() > 1;
                }
            }
            Action::Equalize => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.root.equalize();
                }
            }
            // The Files tab, when open, sits before the first terminal tab in the cycle.
            Action::NextTab => {
                let n = self.tabs.len();
                if self.files_active {
                    if n > 0 {
                        self.show_tab(0);
                    }
                } else if self.active + 1 == n && self.files.is_some() {
                    self.files_active = true;
                } else if n > 0 {
                    self.show_tab((self.active + 1) % n);
                }
            }
            Action::PrevTab => {
                let n = self.tabs.len();
                if self.files_active {
                    if n > 0 {
                        self.show_tab(n - 1);
                    }
                } else if self.active == 0 && self.files.is_some() {
                    self.files_active = true;
                } else if n > 0 {
                    self.show_tab((self.active + n - 1) % n);
                }
            }
            Action::GotoTab(i) => {
                // Like Ghostty, alt+9 is "last tab".
                if i == 8 && !self.tabs.is_empty() {
                    self.show_tab(self.tabs.len() - 1);
                } else if i < self.tabs.len() {
                    self.show_tab(i);
                }
            }
            Action::Copy => {
                if let Some(text) = focused.and_then(|id| self.panes.get(&id)).and_then(|t| t.selection_text()) {
                    ctx.copy_text(text);
                }
            }
            Action::FontBigger | Action::FontSmaller | Action::FontReset => {
                if let Some(id) = focused {
                    let size = self.pane_font(id);
                    let size = match action {
                        Action::FontBigger => Some(size + 1.0),
                        Action::FontSmaller => Some(size - 1.0),
                        _ => None,
                    };
                    self.set_pane_font(ctx, id, size);
                }
            }
            Action::ReloadConfig => {
                self.cfg = Config::load();
                fonts::install(ctx, &self.cfg);
                self.pane_fonts.clear();
                self.update_metrics(ctx);
            }
            Action::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Action::ScrollPageUp | Action::ScrollPageDown | Action::ScrollTop | Action::ScrollBottom => {
                if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                    t.scroll_to(match action {
                        Action::ScrollPageUp => Scroll::PageUp,
                        Action::ScrollPageDown => Scroll::PageDown,
                        Action::ScrollTop => Scroll::Top,
                        _ => Scroll::Bottom,
                    });
                }
            }
            Action::ToggleFiles => self.toggle_files(),
            Action::CloseFiles => self.close_files(),
            Action::ToggleSearch => {
                // Nothing to search in a file browser.
                let focused = focused.filter(|id| !self.browsers.contains_key(id));
                self.search = match (self.search.take(), focused) {
                    // Pressing it again in the same pane closes search; in another pane, moves it.
                    (Some(s), Some(id)) if s.pane != id => Some(Search::new(id)),
                    (Some(_), _) => None,
                    (None, Some(id)) => Some(Search::new(id)),
                    (None, None) => None,
                };
            }
            Action::ClearScreen => {
                // Ask the shell to redraw on a clean screen: clear + form feed (Ctrl+L).
                if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                    t.input(b"\x0c".to_vec());
                }
            }
        }
    }
}
