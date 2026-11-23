//! The on-screen keyboard. winit's view takes the keyboard's typing through `UIKeyInput`
//! alone, and iOS only repeats a held delete key for full text views, so deleting meant
//! tapping once per character. As on Android (`android/java/NexInput.java`), nex puts a view
//! of its own in front of the keyboard: [`NexInput`], a hidden `UITextView` that becomes first
//! responder while nex wants the keyboard. What it types comes back through [`typed_text`].
//!
//! The text view never changes: its overrides hand typing and deletes to nex instead of
//! editing it, and it always holds a space, so the keyboard always has something to delete
//! (and keeps repeating).

use std::cell::OnceCell;
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSInteger, NSObject, NSPoint, NSRect, NSSize, NSString};
use objc2_ui_kit::{UIColor, UIResponder, UIScrollView, UITextView, UIView};

use crate::platform::Typed;

/// Typed or deleted since nex last asked, in order.
static TYPED: Mutex<Vec<Typed>> = Mutex::new(Vec::new());

thread_local! {
    /// The input view, once winit's window is open (main thread only).
    static INPUT: OnceCell<Retained<NexInput>> = const { OnceCell::new() };
}

fn push(typed: Typed) {
    if let Ok(mut q) = TYPED.lock() {
        q.push(typed);
    }
    super::screen::wake();
}

define_class!(
    // SAFETY: UITextView has no subclassing requirements beyond calling super's initializer,
    // and NexInput doesn't implement Drop.
    #[unsafe(super(UITextView, UIScrollView, UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "NexInput"]
    struct NexInput;

    impl NexInput {
        /// The keyboard typed `text` (a key, a word from the predictions, a paste).
        #[unsafe(method(insertText:))]
        fn insert_text(&self, text: &NSString) {
            let text = text.to_string();
            push(match text.as_str() {
                "\n" | "\r" => Typed::Enter,
                _ if text.chars().count() > 1 && text.contains('\n') => Typed::Paste(text),
                _ => Typed::Text { delete: 0, text },
            });
        }

        /// Delete: called again and again while the key is held.
        #[unsafe(method(deleteBackward))]
        fn delete_backward(&self) {
            push(Typed::Text { delete: 1, text: String::new() });
        }

        /// Always something to delete, so the keyboard keeps repeating delete.
        #[unsafe(method(hasText))]
        fn has_text(&self) -> bool {
            true
        }
    }

    unsafe impl NSObjectProtocol for NexInput {}
);

impl NexInput {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: UITextView's designated initializer for a frame.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let clear = UIColor::clearColor();
        this.setBackgroundColor(Some(&clear));
        this.setAlpha(0.02);
        // SAFETY: plain UITextView / UIView / UITextInputTraits setters on a new view. The
        // trait values are the enums' "no" / "none" cases (and the dark keyboard).
        unsafe {
            let _: () = msg_send![&*this, setText: &*NSString::from_str(" ")];
            let _: () = msg_send![&*this, setTextColor: &*clear];
            let _: () = msg_send![&*this, setTintColor: &*clear];
            let _: () = msg_send![&*this, setScrollEnabled: false];
            let _: () = msg_send![&*this, setAutocapitalizationType: 0 as NSInteger];
            let _: () = msg_send![&*this, setAutocorrectionType: 1 as NSInteger];
            let _: () = msg_send![&*this, setSpellCheckingType: 1 as NSInteger];
            let _: () = msg_send![&*this, setSmartQuotesType: 1 as NSInteger];
            let _: () = msg_send![&*this, setSmartDashesType: 1 as NSInteger];
            let _: () = msg_send![&*this, setSmartInsertDeleteType: 1 as NSInteger];
            let _: () = msg_send![&*this, setKeyboardAppearance: 1 as NSInteger];
            // iOS 17's inline predictions would type ahead of the shell.
            if this.respondsToSelector(objc2::sel!(setInlinePredictionType:)) {
                let _: () = msg_send![&*this, setInlinePredictionType: 1 as NSInteger];
            }
        }
        this
    }
}

/// The input view, made and put in winit's window the first time it's needed.
fn input(mtm: MainThreadMarker) -> Option<Retained<NexInput>> {
    INPUT.with(|cell| {
        if let Some(input) = cell.get() {
            return Some(input.clone());
        }
        let window = super::screen::winit_window(mtm)?;
        let input = NexInput::new(mtm);
        window.addSubview(&input);
        Some(cell.get_or_init(|| input).clone())
    })
}

/// Text the on-screen keyboard typed or deleted since the last call, oldest first.
pub fn typed_text() -> Vec<Typed> {
    TYPED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Keep the input view first responder (so the keyboard is up and types into it) while this
/// frame says where text goes (egui's IME output), and let it go when nothing does. Called at
/// the end of every frame. egui-winit hands winit's own view first responder when the
/// keyboard is first wanted; the next frame takes it back.
pub fn sync_soft_keyboard(ctx: &egui::Context) {
    super::screen::remember(ctx);
    let Some(mtm) = MainThreadMarker::new() else { return };
    let want = ctx.output(|o| o.ime.is_some());
    let Some(input) = input(mtm) else { return };
    let focused = input.isFirstResponder();
    if want && !focused {
        input.becomeFirstResponder();
        // egui-winit may take it for winit's view right after this frame; check again.
        ctx.request_repaint();
    } else if !want && focused {
        input.resignFirstResponder();
    }
}
