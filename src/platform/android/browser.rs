//! Opening a link in the phone's default browser (or whichever app handles its scheme):
//! an `ACTION_VIEW` intent for the URI, started from the activity.

use jni::objects::{JObject, JValue};

use super::clipboard::with_context;

/// `Intent.FLAG_ACTIVITY_NEW_TASK`: the browser opens in its own task, so Back from it
/// returns to nex rather than stacking the browser inside nex's task.
const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;

/// Hand `uri` to Android. When no app takes it (`ActivityNotFoundException`), say so on
/// logcat; nothing else happens.
pub fn open_uri(uri: &std::ffi::OsStr) {
    let uri = uri.to_string_lossy();
    let opened = with_context(|env, context| {
        let text = env.new_string(&*uri)?;
        let parsed = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&text)])?.l()?;
        let action = env.get_static_field("android/content/Intent", "ACTION_VIEW", "Ljava/lang/String;")?.l()?;
        let intent: JObject =
            env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&parsed)])?;
        env.call_method(&intent, "addFlags", "(I)Landroid/content/Intent;", &[JValue::Int(FLAG_ACTIVITY_NEW_TASK)])?;
        env.call_method(context, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
        Ok(())
    });
    if opened.is_none() {
        eprintln!("nex: no app on this phone opened {uri}");
    }
}
