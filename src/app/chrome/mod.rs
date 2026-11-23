//! Window chrome: the title bar and tab strip, the sidebar, and window-level edges and drops.
//! Here, what the title bar and the sidebar both show: the Files / Agents tab's name.

pub(super) mod keyboard_button;
pub(super) mod sidebar;
pub(super) mod titlebar;
pub(super) mod window;

use std::borrow::Cow;

use crate::app::App;

impl App {
    /// The name on the Files / Agents tab: with `files` (it is a file explorer), the folder
    /// the focused pane is in (`~` if there is none to name), else "Agents".
    fn home_label(&self, files: bool) -> Cow<'static, str> {
        if !files {
            return Cow::Borrowed("Agents");
        }
        self.focused_cwd()
            .and_then(|p| p.file_name().map(|n| Cow::Owned(n.to_string_lossy().into_owned())))
            .unwrap_or(Cow::Borrowed("~"))
    }
}
