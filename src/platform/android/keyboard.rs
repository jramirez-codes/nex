//! The on-screen keyboard. NativeActivity gives the keyboard no InputConnection, so text it
//! commits in one go (a paste from its clipboard history, a suggestion, an emoji) arrives as a
//! key event whose characters the NDK can't read, and is lost. nex adds a view of its own that
//! the keyboard types into (`java/NexInput.java`, compiled to a dex by `build.rs` and loaded
//! here): what the keyboard types (glide typing included, with a space after each glided word:
//! see [`AutoSpace`]) or commits comes back through [`typed_text`]. Enter, and Backspace with
//! nothing composing, still arrive as key events through winit.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, NativeMethod};

use super::clipboard::with_context;
use crate::platform::{AutoSpace, Haptic, Typed};

/// The activity (ndk-context's context is the Application, which can't host a view).
fn activity() -> Option<JObject<'static>> {
    let ptr = super::APP.get()?.activity_as_ptr();
    // SAFETY: a global reference android-activity holds for as long as the app runs; wrapping
    // it doesn't take ownership.
    (!ptr.is_null()).then(|| unsafe { JObject::from_raw(ptr.cast()) })
}

/// The Java class, compiled by `build.rs`.
static DEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/classes.dex"));
const CLASS: &str = "io.github.jramirez_codes.nex.NexInput";

/// `NexInput`, once loaded (a class from our own loader can't be found by name later).
static INPUT_CLASS: OnceLock<GlobalRef> = OnceLock::new();
/// Text typed or committed since nex last asked, in order.
static COMMITTED: Mutex<Vec<Typed>> = Mutex::new(Vec::new());
/// The word the keyboard is composing, for the space after a glided one.
static WORD: Mutex<AutoSpace> = Mutex::new(AutoSpace::new());
/// Woken when text arrives, so a frame reads it.
static CTX: OnceLock<egui::Context> = OnceLock::new();
/// Whether nex last asked for the keyboard to be shown.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// What the system last said about the keyboard: 0 nothing yet, 1 down, 2 up.
static VISIBLE: AtomicU8 = AtomicU8::new(0);

/// `NexInput.imeVisible`: the keyboard came up or went away. Called on Android's UI thread.
extern "system" fn ime_visible(_env: JNIEnv, _class: JClass, visible: jni::sys::jboolean) {
    VISIBLE.store(if visible != 0 { 2 } else { 1 }, Ordering::Relaxed);
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// Whether the on-screen keyboard is up, as far as the system has said (`None` before it has).
pub fn soft_keyboard_visible() -> Option<bool> {
    match VISIBLE.load(Ordering::Relaxed) {
        0 => None,
        v => Some(v == 2),
    }
}

fn push(typed: Typed) {
    if let Ok(mut q) = COMMITTED.lock() {
        q.push(typed);
    }
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// `NexInput.commit`: the keyboard pasted `text`. Called on Android's UI thread.
extern "system" fn commit(mut env: JNIEnv, _class: JClass, text: JString) {
    let Ok(text) = env.get_string(&text) else { return };
    push(Typed::Paste(text.into()));
}

/// `NexInput.type`: the keyboard took back `delete` characters and typed `text`, into the
/// word it's composing if `composing`. Called on Android's UI thread.
extern "system" fn type_text(mut env: JNIEnv, _class: JClass, delete: jni::sys::jint, text: JString, composing: jni::sys::jboolean) {
    let Ok(text) = env.get_string(&text) else { return };
    let (delete, text): (usize, String) = (delete.max(0) as usize, text.into());
    // Less a space right after the one nex typed after a glided word.
    let text = match WORD.lock() {
        Ok(mut word) => word.typed(delete, &text, composing != 0).to_owned(),
        Err(_) => text,
    };
    if delete > 0 || !text.is_empty() {
        push(Typed::Text { delete, text });
    }
}

/// `NexInput.wordDone`: the keyboard `committed` the word it was composing, or gave it up. A
/// glided word gets a space after it. Called on Android's UI thread.
extern "system" fn word_done(_env: JNIEnv, _class: JClass, committed: jni::sys::jboolean) {
    let space = WORD.lock().is_ok_and(|mut word| word.word_done(committed != 0));
    if space {
        push(Typed::Text { delete: 0, text: " ".into() });
    }
}

/// Load `NexInput` from the built-in dex, hook up its native method and add its view to the
/// activity. Run once at start; on failure typing still works, only committed text is lost.
pub fn install() {
    let Some(activity) = activity() else { return };
    let activity = &activity;
    let loaded = with_context(|env, _app| {
        // InMemoryDexClassLoader reads the buffer while it loads; it lives as long as the app.
        let bytes: &'static mut [u8] = Box::leak(DEX.to_vec().into_boxed_slice());
        // SAFETY: the buffer is leaked above, so it outlives the ByteBuffer.
        let buffer = unsafe { env.new_direct_byte_buffer(bytes.as_mut_ptr(), bytes.len()) }?;
        let parent = env.call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
        let loader = env.new_object(
            "dalvik/system/InMemoryDexClassLoader",
            "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
            &[JValue::Object(&buffer), JValue::Object(&parent)],
        )?;
        let name = env.new_string(CLASS)?;
        let class: JClass = env
            .call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?
            .l()?
            .into();
        env.register_native_methods(
            &class,
            &[
                NativeMethod { name: "commit".into(), sig: "(Ljava/lang/String;)V".into(), fn_ptr: commit as *mut c_void },
                NativeMethod { name: "type".into(), sig: "(ILjava/lang/String;Z)V".into(), fn_ptr: type_text as *mut c_void },
                NativeMethod { name: "wordDone".into(), sig: "(Z)V".into(), fn_ptr: word_done as *mut c_void },
                NativeMethod { name: "imeVisible".into(), sig: "(Z)V".into(), fn_ptr: ime_visible as *mut c_void },
            ],
        )?;
        env.call_static_method(&class, "install", "(Landroid/app/Activity;)V", &[JValue::Object(activity)])?;
        let global = env.new_global_ref(&class)?;
        Ok(global)
    });
    match loaded {
        Some(class) => {
            let _ = INPUT_CLASS.set(class);
        }
        None => eprintln!("nex: couldn't add the keyboard's input view; pastes from the keyboard won't arrive"),
    }
}

/// Text the on-screen keyboard typed or pasted since the last call, oldest first.
pub fn typed_text() -> Vec<Typed> {
    COMMITTED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Show the keyboard while this frame says where text goes (egui's IME output), and hide it
/// when nothing does. Called at the end of every frame.
pub fn sync_soft_keyboard(ctx: &egui::Context) {
    let _ = CTX.get_or_init(|| ctx.clone());
    let want = ctx.output(|o| o.ime.is_some());
    if SHOWN.swap(want, Ordering::Relaxed) == want {
        return;
    }
    let (Some(class), Some(activity)) = (INPUT_CLASS.get(), activity()) else { return };
    let activity = &activity;
    with_context(|env, _app| {
        let class: &JClass = class.as_obj().into();
        env.call_static_method(class, "showKeyboard", "(Landroid/app/Activity;Z)V", &[JValue::Object(activity), JValue::Bool(want.into())])?;
        Ok(())
    });
}

/// Play a haptic tick through the input view (`View.performHapticFeedback`).
pub fn haptic(kind: Haptic) {
    // HapticFeedbackConstants.
    let constant = match kind {
        Haptic::Tap => 3,      // KEYBOARD_TAP
        Haptic::Tick => 4,     // CLOCK_TICK
        Haptic::Confirm => 16, // CONFIRM (LONG_PRESS before Android 11, in NexInput)
        Haptic::Release => 1,  // VIRTUAL_KEY
    };
    let (Some(class), Some(activity)) = (INPUT_CLASS.get(), activity()) else { return };
    let activity = &activity;
    with_context(|env, _app| {
        let class: &JClass = class.as_obj().into();
        env.call_static_method(class, "haptic", "(Landroid/app/Activity;I)V", &[JValue::Object(activity), JValue::Int(constant)])?;
        Ok(())
    });
}
