//! Centers the system traffic lights in nex's own (taller) title bar.
//!
//! With a full-size content view, AppKit places the buttons for its standard 28pt title bar,
//! which sits above the middle of nex's header row. We stretch the title bar container to the
//! header's height and center each button in it. AppKit puts them back on resize and when
//! leaving full screen, so this runs every frame and only touches views that moved.
//!
//! Setting the window title puts them back too, so nex sets it here and re-centers the
//! buttons in the same call, before AppKit draws the title bar.

use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowButton};
use objc2_foundation::{NSPoint, NSRect, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Center the close / minimize / zoom buttons in the top `height` points of the window.
pub fn align_window_buttons(frame: &eframe::Frame, height: f32) {
    if let Some(window) = ns_window(frame) {
        align(&window, height as f64);
    }
}

/// Set the window title. With `buttons_height`, the traffic lights stay centered in that
/// height: AppKit lays the title bar out again for the new title, which would otherwise
/// show them at their default spot until the next frame.
pub fn set_window_title(ctx: &egui::Context, frame: &eframe::Frame, title: &str, buttons_height: Option<f32>) {
    let Some(window) = ns_window(frame) else {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_owned()));
        return;
    };
    window.setTitle(&NSString::from_str(title));
    if let Some(height) = buttons_height {
        // SAFETY: as in `align`, these are the window's own views.
        if let Some(frame_view) = window.contentView().and_then(|view| unsafe { view.superview() }) {
            frame_view.layoutSubtreeIfNeeded();
        }
        align(&window, height as f64);
    }
}

fn ns_window(frame: &eframe::Frame) -> Option<Retained<NSWindow>> {
    let handle = frame.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: winit's handle points at the window's live content view, and eframe calls
    // `update` on the main thread.
    let view: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
    view.window()
}

fn align(window: &NSWindow, height: f64) {
    let buttons = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton]
        .map(|kind| window.standardWindowButton(kind));
    let Some(close) = &buttons[0] else { return };
    // close → NSTitlebarView → NSTitlebarContainerView → the window's frame view.
    // SAFETY: `superview` is only unsafe because a view may be released mid-mutation of
    // the view tree; these are AppKit's own title bar views, alive as long as the window.
    let Some(container) = (unsafe { close.superview().and_then(|bar| bar.superview()) }) else { return };
    let Some(frame_view) = (unsafe { container.superview() }) else { return };

    let mut f = container.frame();
    let top = frame_view.bounds().size.height;
    if f.size.height != height || f.origin.y != top - height {
        f.size.height = height;
        f.origin.y = top - height;
        container.setFrame(f);
    }
    for button in buttons.iter().flatten() {
        let b: NSRect = button.frame();
        let y = ((height - b.size.height) / 2.0).round();
        if b.origin.y != y {
            button.setFrameOrigin(NSPoint::new(b.origin.x, y));
        }
    }
}
