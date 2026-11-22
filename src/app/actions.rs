//! Everything the user can ask for (keyboard shortcuts, buttons), and carrying it out.
//! Which keys ask for what is in `input/keymap.rs`.

use alacritty_terminal::grid::Scroll;
use egui::ViewportCommand;

use crate::config::{FONT_MAX, FONT_MIN};
use crate::layout::{self, Axis, Direction, PaneId, PaneKind};
use crate::terminal::search::Search;

use crate::app::App;
use crate::app::ssh::follow::Place;
use crate::app::input::zoom::EXPLORER_ZOOM_STEP;

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
    /// Swap the focused pane with its neighbor in this direction.
    Swap(Direction),
    /// Swap two panes of a tab (a header dropped on the middle of another pane).
    SwapPanes(PaneId, PaneId),
    /// Move a pane to this side of another (a header dropped on its edge), even across tabs.
    MovePane(PaneId, PaneId, Direction),
    /// Move a pane into the tab with this uid, beside its focused pane.
    PaneToTab(PaneId, u64),
    /// Move a pane out of its tab into a new one of its own.
    PaneToNewTab(PaneId),
    /// Drop a pane being dragged by its header back where it was.
    CancelPaneDrag,
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
    /// Open a Changes (git diff) pane beside the focused pane, or close the tab's one.
    ToggleGitDiff,
    /// Open an Agents pane beside the focused pane, or close the tab's one.
    ToggleAgents,
    /// Show the Agents tab (the title bar's Agents label; opening it the first time), or
    /// from it, go back to the terminal tab.
    ToggleAgentsTab,
    /// Share this machine's microphone with a pane's ssh host, or stop; `ToggleMic(0)` means
    /// the focused pane (see `ssh/mic.rs`).
    ToggleMic(PaneId),
    /// Make a pane the sticky one, which follows the user to every tab, or unpin it
    /// (Shift + the header's ×).
    ToggleSticky(PaneId),
}

impl App {
    pub(super) fn apply(&mut self, ctx: &egui::Context, action: Action) {
        let focused = self.focused_pane();
        match action {
            Action::NewTab => self.open_from(ctx, focused, Place::Tab),
            Action::NewWindow => self.new_window(),
            Action::ClosePane(0) => {
                if let Some(id) = focused {
                    self.user_closing();
                    self.close_pane(ctx, id)
                }
            }
            Action::ClosePane(id) => {
                self.user_closing();
                self.close_pane(ctx, id)
            }
            Action::CloseTab(i) => {
                self.user_closing();
                self.close_tab(i)
            }
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
            Action::Swap(dir) => {
                if let Some(id) = focused
                    && let Some(next) = layout::neighbor(&self.card_rects, id, dir) {
                        self.swap_panes(ctx, id, next);
                    }
            }
            Action::SwapPanes(a, b) => self.swap_panes(ctx, a, b),
            Action::MovePane(pane, target, dir) => self.move_pane(ctx, pane, target, dir),
            Action::PaneToTab(pane, uid) => self.pane_to_tab(ctx, pane, uid),
            Action::PaneToNewTab(pane) => self.pane_to_new_tab(ctx, pane),
            Action::CancelPaneDrag => self.cancel_pane_drag(ctx),
            Action::ToggleZoom => self.toggle_zoom(ctx, None),
            Action::ToggleZoomPane(id) => self.toggle_zoom(ctx, Some(id)),
            Action::Equalize => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.root.equalize();
                    tab.motion.kick_reflow(ctx);
                }
            }
            Action::NextTab => self.cycle_tab(true),
            Action::PrevTab => self.cycle_tab(false),
            Action::GotoTab(i) => self.goto_tab(i),
            Action::Copy => {
                if let Some(text) = focused.and_then(|id| self.panes.get(&id)).and_then(|t| t.selection_text()) {
                    self.clipboard_toast(ctx, "Copied", &text);
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
            Action::ToggleGitDiff => self.toggle_git_diff(ctx),
            Action::ToggleAgents => self.toggle_agents(ctx),
            Action::ToggleAgentsTab => self.toggle_agents_tab(),
            Action::ToggleMic(0) => {
                if let Some(id) = focused {
                    self.toggle_mic(ctx, id)
                }
            }
            Action::ToggleMic(id) => self.toggle_mic(ctx, id),
            Action::ToggleSticky(id) => self.toggle_sticky(ctx, id),
            Action::ExploreHere => {
                if let Some(id) = focused {
                    self.split_explorer(ctx, id, Axis::Horizontal)
                }
            }
            Action::CloseExplorer => {
                self.user_closing();
                self.close_explorer()
            }
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
    /// The card and its text scale between the slot and the whole tab.
    fn toggle_zoom(&mut self, ctx: &egui::Context, pane: Option<PaneId>) {
        let Some(tab) = self.cur_tab_mut() else { return };
        if let Some(id) = pane {
            tab.focused = id;
        }
        let zoomed = !tab.zoomed && tab.root.panes().len() > 1;
        if zoomed == tab.zoomed {
            return;
        }
        let focused = tab.focused;
        let ids = tab.root.panes();
        // Zooming in, the other panes fade out from their last frame.
        let others = if zoomed {
            ids.into_iter()
                .filter(|&id| id != focused)
                .map(|id| {
                    let (title, process) = self.pane_label(id);
                    let shapes = self.panes.get(&id).map(|t| t.shapes()).unwrap_or_default();
                    (id, title, process, shapes)
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some(tab) = self.cur_tab_mut() {
            tab.zoomed = zoomed;
            tab.motion.kick_zoom(ctx, focused, zoomed, others);
        }
    }

    /// Next/previous tab. The Files tab, when open, sits before the first terminal tab in
    /// the cycle.
    fn cycle_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        let at_edge = if forward { self.active + 1 == n } else { self.active == 0 };
        if self.home_active {
            if n > 0 {
                self.show_tab(if forward { 0 } else { n - 1 });
            }
        } else if at_edge && self.home_tab.is_some() {
            self.home_active = true;
        } else if n > 0 {
            self.show_tab(if forward { (self.active + 1) % n } else { (self.active + n - 1) % n });
        }
    }

    /// Go to tab `i` (Alt+1..9, Cmd+1..9 on macOS); the 9th means "last tab".
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
        match self.pane_kind(id) {
            Some(PaneKind::Explorer) => {
                let explorer = self.explorers.get_mut(&id).unwrap();
                match action {
                    Action::FontBigger => explorer.zoom_by(EXPLORER_ZOOM_STEP, None),
                    Action::FontSmaller => explorer.zoom_by(1.0 / EXPLORER_ZOOM_STEP, None),
                    _ => explorer.reset_zoom(),
                }
            }
            Some(PaneKind::Agents) => {}
            Some(PaneKind::Diff) => {
                let size = self.pane_font(id);
                match action {
                    Action::FontBigger => self.pane_fonts.insert(id, (size + 1.0).min(FONT_MAX)),
                    Action::FontSmaller => self.pane_fonts.insert(id, (size - 1.0).max(FONT_MIN)),
                    _ => self.pane_fonts.remove(&id),
                };
            }
            Some(PaneKind::Terminal) | None => {
                let size = self.pane_font(id);
                let size = match action {
                    Action::FontBigger => Some(size + 1.0),
                    Action::FontSmaller => Some(size - 1.0),
                    _ => None,
                };
                self.set_pane_font(ctx, id, size);
            }
        }
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
        let focused = self.focused_pane().filter(|id| self.panes.contains_key(id));
        self.search = match (self.search.take(), focused) {
            (Some(s), Some(id)) if s.pane != id => Some(Search::new(id)),
            (Some(_), _) => None,
            (None, Some(id)) => Some(Search::new(id)),
            (None, None) => None,
        };
    }
}
