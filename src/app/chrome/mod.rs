//! Window chrome: the title bar and tab strip, the sidebar, and window-level edges and drops.
//! Here, the chrome's state `App` keeps between frames ([`Chrome`]), and what the title bar and
//! the sidebar both show: the Files / Agents tab's name.

pub(super) mod keyboard_button;
pub(super) mod sidebar;
pub(super) mod titlebar;
pub(super) mod window;

use std::borrow::Cow;
use std::collections::HashMap;

use crate::app::App;
use crate::app::chrome::keyboard_button::KeyboardButton;
use crate::app::chrome::titlebar::{TabScroll, TabStrip};
use crate::ui::glide::Glide;
use crate::ui::toast::Toast;

/// What the window draws around the panes, kept across frames: the tab strip, the keyboard
/// button, toasts, and the window title. Kept by `App` as `chrome`.
pub(in crate::app) struct Chrome {
    /// Where the tabs were drawn and how they are animating (see `chrome/titlebar.rs`).
    pub(in crate::app) strip: TabStrip,
    /// The floating button that shows or hides the on-screen keyboard.
    pub(in crate::app) keyboard_button: KeyboardButton,
    /// Typing goes through an on-screen keyboard (`platform::SOFT_KEYBOARD`; tests turn it on).
    pub(in crate::app) soft_keyboard: bool,
    /// The "Copied …" / "Pasted …" message currently on screen.
    pub(in crate::app) toast: Option<Toast>,
    /// A toast to show on the next frame, from code without the egui context.
    pub(in crate::app) notice: Option<String>,
    pub(in crate::app) window_title: String,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            strip: TabStrip {
                pill_glide: Glide::new(0.2),
                tab_rects: HashMap::new(),
                tab_ghosts: Vec::new(),
                files_rect: None,
                dog_wake: None,
                dog_was_active: false,
                dog_rect: None,
                tab_hits: Vec::new(),
                strip_rect: None,
                scroll: TabScroll::new(crate::platform::WINDOW.scroll_tabs),
                badge: None,
            },
            keyboard_button: KeyboardButton::default(),
            soft_keyboard: crate::platform::SOFT_KEYBOARD,
            toast: None,
            notice: None,
            window_title: String::new(),
        }
    }
}

impl App {
    /// The name on the Files / Agents tab: with `files` (it is a file explorer), the folder
    /// the focused pane is in (`~` if there is none to name), else "Agents".
    fn home_label(&self, files: bool) -> Cow<'static, str> {
        if !files {
            return Cow::Borrowed("Agents");
        }
        self.focused_cwd().and_then(|p| p.file_name().map(|n| Cow::Owned(n.to_string_lossy().into_owned()))).unwrap_or(Cow::Borrowed("~"))
    }
}
