//! Names nex's application menu "Nex".
//!
//! winit builds the menu bar with an untitled first item, and on recent macOS the bar can
//! show that item's class name ("NSMenuItem") instead of the app's name. Its About / Hide /
//! Quit entries also use the process name, lowercase "nex". This titles the item and its
//! menu and fixes those entries up.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

/// The name shown in the menu bar and the app menu.
const APP_NAME: &str = "Nex";

/// Title the app menu. Cheap and idempotent, so it can run whenever the window title changes.
pub fn name_app_menu() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let Some(item) = app.mainMenu().and_then(|bar| bar.itemAtIndex(0)) else { return };
    let name = NSString::from_str(APP_NAME);
    if item.title().to_string() != APP_NAME {
        item.setTitle(&name);
    }
    let Some(menu) = item.submenu() else { return };
    if menu.title().to_string() != APP_NAME {
        menu.setTitle(&name);
    }
    for i in 0..menu.numberOfItems() {
        let Some(entry) = menu.itemAtIndex(i) else { continue };
        let title = entry.title().to_string();
        if let Some(verb) = title.strip_suffix(" nex") {
            entry.setTitle(&NSString::from_str(&format!("{verb} {APP_NAME}")));
        }
    }
}
