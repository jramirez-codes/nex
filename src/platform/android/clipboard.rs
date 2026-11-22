//! The system clipboard, through Android's `ClipboardManager` (JNI). egui has no clipboard of
//! its own on Android, only one inside the app, so nex reads and writes this one itself:
//! text fields paste from it, and whatever nex copies is mirrored to it.

use jni::JavaVM;
use jni::objects::{JObject, JString, JValue};

/// Run `f` with the JNI environment and the app's `Context`. `None` if anything along the way
/// failed (a Java exception is cleared, not left pending).
pub(super) fn with_context<T>(f: impl FnOnce(&mut jni::JNIEnv, &JObject) -> jni::errors::Result<T>) -> Option<T> {
    let ctx = ndk_context::android_context();
    // SAFETY: android-activity hands ndk-context the process's JavaVM and the activity, both
    // valid for as long as the app runs.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    // SAFETY: a global reference android-activity holds; wrapping it doesn't take ownership.
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };
    let mut env = vm.attach_current_thread().ok()?;
    // A local frame, so the references made here are freed even on a thread that stays attached.
    let result = env.with_local_frame(16, |env| f(env, &context));
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    result.ok()
}

fn manager<'a>(env: &mut jni::JNIEnv<'a>, context: &JObject) -> jni::errors::Result<JObject<'a>> {
    let name = env.new_string("clipboard")?;
    env.call_method(context, "getSystemService", "(Ljava/lang/String;)Ljava/lang/Object;", &[JValue::Object(&name)])?.l()
}

/// The text on the system clipboard, if it holds any.
pub fn clipboard_text() -> Option<String> {
    with_context(|env, context| {
        let cm = manager(env, context)?;
        let clip = env.call_method(&cm, "getPrimaryClip", "()Landroid/content/ClipData;", &[])?.l()?;
        if clip.is_null() || env.call_method(&clip, "getItemCount", "()I", &[])?.i()? < 1 {
            return Ok(None);
        }
        let item = env.call_method(&clip, "getItemAt", "(I)Landroid/content/ClipData$Item;", &[JValue::Int(0)])?.l()?;
        let text = env.call_method(&item, "coerceToText", "(Landroid/content/Context;)Ljava/lang/CharSequence;", &[JValue::Object(context)])?.l()?;
        if text.is_null() {
            return Ok(None);
        }
        let s = JString::from(env.call_method(&text, "toString", "()Ljava/lang/String;", &[])?.l()?);
        let s: String = env.get_string(&s)?.into();
        Ok(Some(s))
    })
    .flatten()
    .filter(|s| !s.is_empty())
}

/// Put `text` on the system clipboard.
pub fn set_clipboard_text(text: &str) {
    with_context(|env, context| {
        let cm = manager(env, context)?;
        let (label, text) = (env.new_string("nex")?, env.new_string(text)?);
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[JValue::Object(&label), JValue::Object(&text)],
            )?
            .l()?;
        env.call_method(&cm, "setPrimaryClip", "(Landroid/content/ClipData;)V", &[JValue::Object(&clip)])?;
        Ok(())
    });
}

/// Mirror what nex copied this frame (egui's copy commands) to the system clipboard.
pub fn mirror_copies(ctx: &egui::Context) {
    let copied: Vec<String> = ctx.output(|o| {
        o.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect()
    });
    if let Some(text) = copied.last() {
        set_clipboard_text(text);
    }
}
