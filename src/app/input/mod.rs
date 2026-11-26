//! Input: keyboard shortcuts and typing, the keymap tables, the mouse over terminals,
//! swiping between tabs, and zoom. Here, the input state `App` keeps between frames
//! ([`InputState`]); `press` holds the press and hold timings the touch controls share.

pub(super) mod keyboard;
pub(super) mod keymap;
pub(super) mod mouse;
pub(super) mod press;
pub(super) mod swipe;
pub(super) mod zoom;

use egui::Pos2;

use crate::app::input::mouse::MouseState;
use crate::app::input::swipe::TabSwipe;
use crate::app::workspace::pane_drag::PaneDrag;
use crate::layout::PaneId;
use crate::terminal::search::{Link, LinkFinder, Search};

/// What the pointer, fingers and search are in the middle of, kept across frames. Kept by
/// `App` as `input`.
pub(in crate::app) struct InputState {
    pub(in crate::app) mouse: MouseState,
    /// A finger on a pane that may become a sideways swipe to the next or previous tab.
    pub(in crate::app) tab_swipe: Option<TabSwipe>,
    /// Pinch zoom of a terminal not yet applied (font sizes change in whole points).
    pub(in crate::app) pinch_accum: f32,
    /// The zoomed pane of the shown tab and its font size: scaled up from its own size so it
    /// keeps its grid while filling the tab.
    pub(in crate::app) zoom_font: Option<(PaneId, f32)>,
    /// Last known pointer position (touch releases report the pointer as gone).
    pub(in crate::app) last_pointer: Option<Pos2>,
    /// A pane being dragged by its header, if any (it can be carried to another tab).
    pub(in crate::app) pane_drag: Option<PaneDrag>,
    /// Link under the pointer while Ctrl is held.
    pub(in crate::app) hover_link: Option<(PaneId, Link)>,
    pub(in crate::app) links: LinkFinder,
    /// Opens a Ctrl+clicked or tapped link: the platform's browser (`platform::open_uri`),
    /// swapped for a recorder in tests.
    pub(in crate::app) open_link: fn(&std::ffi::OsStr),
    pub(in crate::app) search: Option<Search>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            mouse: MouseState::default(),
            tab_swipe: None,
            pinch_accum: 1.0,
            zoom_font: None,
            last_pointer: None,
            pane_drag: None,
            hover_link: None,
            links: LinkFinder::new(),
            open_link: crate::platform::open_uri,
            search: None,
        }
    }
}
