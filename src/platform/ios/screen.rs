//! The screen area nex may draw in.
//!
//! winit sizes an iOS window to its safe area (clear of the notch and home indicator) but
//! draws it over the whole screen and reports touches in whole-screen coordinates, so the
//! picture would be stretched and taps land off by the notch's height. Instead, as on Android,
//! nex's window covers the whole screen (winit's window is told its safe area is empty) and
//! [`content_rect_px`] says which part the notch, the home indicator and the on-screen
//! keyboard leave free.

use std::ffi::CStr;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::Mutex;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Imp, NSObjectProtocol, Sel};
use objc2::{ClassType, MainThreadMarker, msg_send, sel};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSValue};
use objc2_ui_kit::{
    UIApplication, UIEdgeInsets, UIKeyboardFrameEndUserInfoKey, UIKeyboardWillChangeFrameNotification, UIKeyboardWillHideNotification, UIView,
    UIWindow,
};

/// winit's window class (`winit/src/platform_impl/ios/window.rs`).
const WINIT_WINDOW: &CStr = c"WinitUIWindow";

/// `-[UIWindow safeAreaInsets]` as UIKit implements it, for the real insets.
static REAL_SAFE_AREA: OnceLock<Imp> = OnceLock::new();
type SafeAreaFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> UIEdgeInsets;

/// The keyboard's top edge, in points from the top of the screen, while it's up.
static KEYBOARD_TOP: Mutex<Option<f64>> = Mutex::new(None);
/// What the system last said about the keyboard: `None` nothing yet.
static KEYBOARD_UP: Mutex<Option<bool>> = Mutex::new(None);
/// Woken when the keyboard moves, so a frame lays nex out again.
static CTX: OnceLock<egui::Context> = OnceLock::new();

/// Our `-[UIWindow safeAreaInsets]`: none for winit's window, UIKit's own for every other
/// window (the keyboard's included).
unsafe extern "C-unwind" fn safe_area_insets(this: *mut AnyObject, cmd: Sel) -> UIEdgeInsets {
    // SAFETY: `this` is a live UIWindow receiving a message.
    let winit = AnyClass::get(WINIT_WINDOW).is_some_and(|cls| unsafe { msg_send![&*this, isKindOfClass: cls] });
    if winit {
        return UIEdgeInsets { top: 0.0, left: 0.0, bottom: 0.0, right: 0.0 };
    }
    real_insets(this, cmd)
}

fn real_insets(this: *mut AnyObject, cmd: Sel) -> UIEdgeInsets {
    match REAL_SAFE_AREA.get() {
        // SAFETY: UIKit's own implementation of the same method, called the same way.
        Some(imp) => unsafe { std::mem::transmute::<Imp, SafeAreaFn>(*imp)(this, cmd) },
        None => UIEdgeInsets { top: 0.0, left: 0.0, bottom: 0.0, right: 0.0 },
    }
}

/// Swap in [`safe_area_insets`] and start following the keyboard. Run once, before the
/// window opens.
pub(super) fn install() {
    let class = UIWindow::class();
    let sel = sel!(safeAreaInsets);
    let Some(method) = class.instance_method(sel) else { return };
    // SAFETY: the replacement has the method's signature (`-(UIEdgeInsets)safeAreaInsets`).
    let old = unsafe { method.set_implementation(std::mem::transmute::<SafeAreaFn, Imp>(safe_area_insets)) };
    let _ = REAL_SAFE_AREA.set(old);
    observe_keyboard();
}

fn observe_keyboard() {
    let center = NSNotificationCenter::defaultCenter();
    let moved = RcBlock::new(|note: NonNull<NSNotification>| {
        // SAFETY: the notification center passes a live notification.
        let note = unsafe { note.as_ref() };
        let top = note.userInfo().and_then(|info| {
            // SAFETY: a UIKit constant.
            let value = info.objectForKey(unsafe { UIKeyboardFrameEndUserInfoKey })?;
            let value: Retained<NSValue> = value.downcast().ok()?;
            // SAFETY: the keyboard's end frame is an NSValue holding a CGRect.
            let rect: objc2_foundation::NSRect = unsafe { msg_send![&*value, CGRectValue] };
            Some(rect)
        });
        let screen_h = MainThreadMarker::new().and_then(winit_window).map(|w| w.bounds().size.height);
        let up = matches!((top, screen_h), (Some(r), Some(h)) if r.origin.y < h && r.size.height > 0.0);
        *KEYBOARD_TOP.lock().unwrap() = if up { top.map(|r| r.origin.y) } else { None };
        *KEYBOARD_UP.lock().unwrap() = Some(up);
        if let Some(ctx) = CTX.get() {
            ctx.request_repaint();
        }
    });
    let hidden = RcBlock::new(|_note: NonNull<NSNotification>| {
        *KEYBOARD_TOP.lock().unwrap() = None;
        *KEYBOARD_UP.lock().unwrap() = Some(false);
        if let Some(ctx) = CTX.get() {
            ctx.request_repaint();
        }
    });
    // SAFETY: UIKit constants; the blocks live as long as the observers (the whole app).
    unsafe {
        let a = center.addObserverForName_object_queue_usingBlock(Some(UIKeyboardWillChangeFrameNotification), None, None, &moved);
        let b = center.addObserverForName_object_queue_usingBlock(Some(UIKeyboardWillHideNotification), None, None, &hidden);
        // The observers are kept for the app's lifetime.
        std::mem::forget((a, b));
    }
}

/// winit's window, once it's open.
pub(super) fn winit_window(mtm: MainThreadMarker) -> Option<Retained<UIWindow>> {
    let cls = AnyClass::get(WINIT_WINDOW)?;
    #[allow(deprecated)]
    let windows = UIApplication::sharedApplication(mtm).windows();
    windows.iter().find(|w| w.isKindOfClass(cls))
}

/// The part of the window, in pixels, that the notch, the home indicator and the keyboard
/// leave free: `[left, top, right, bottom]`.
pub fn content_rect_px() -> Option<[f32; 4]> {
    let mtm = MainThreadMarker::new()?;
    let window = winit_window(mtm)?;
    let view: &UIView = &window;
    let ptr = view as *const UIView as *mut AnyObject;
    let insets = real_insets(ptr, sel!(safeAreaInsets));
    let bounds = window.bounds();
    // Points to pixels: the screen's scale, which winit's view and so egui use too (the
    // window's own `contentScaleFactor` stays 1).
    let scale = window.screen().scale();
    let mut bottom = bounds.size.height - insets.bottom;
    if let Some(top) = *KEYBOARD_TOP.lock().unwrap() {
        bottom = bottom.min(top);
    }
    let (l, t, r) = (insets.left, insets.top, bounds.size.width - insets.right);
    (r > l && bottom > t).then(|| [l, t, r, bottom].map(|v| (v * scale) as f32))
}

/// Whether the on-screen keyboard is up, as far as the system has said (`None` before it has).
pub fn soft_keyboard_visible() -> Option<bool> {
    *KEYBOARD_UP.lock().unwrap()
}

/// Keep the context, to wake a frame when the keyboard moves or types.
pub(super) fn remember(ctx: &egui::Context) {
    let _ = CTX.get_or_init(|| ctx.clone());
}

/// Ask for a frame, to read what changed.
pub(super) fn wake() {
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}
