//! The phone's Downloads folder, where the Files pane saves a file downloaded from the ssh
//! host (see `file_explorer/fs/download.rs`). From Android 11 an app may create files there
//! by path without asking for a storage permission, and the system adds them to the
//! Downloads collection (they show in the Files app); older versions refuse the write, and
//! the explorer shows why.

use std::path::PathBuf;
use std::sync::OnceLock;

use jni::objects::{JObject, JString, JValue};

use super::clipboard::with_context;

/// The shared Downloads folder (`/storage/emulated/0/Download` for the phone's main user),
/// as Android names it. Asked once.
pub fn downloads_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = with_context(|env, _context| {
            let kind = env.get_static_field("android/os/Environment", "DIRECTORY_DOWNLOADS", "Ljava/lang/String;")?.l()?;
            let file = env
                .call_static_method(
                    "android/os/Environment",
                    "getExternalStoragePublicDirectory",
                    "(Ljava/lang/String;)Ljava/io/File;",
                    &[JValue::Object(&kind)],
                )?
                .l()?;
            if file.is_null() {
                return Ok(None);
            }
            let path: JObject = env.call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])?.l()?;
            let path = JString::from(path);
            Ok(Some(PathBuf::from(String::from(env.get_string(&path)?))))
        })
        .flatten();
        if dir.is_none() {
            eprintln!("nex: Android didn't say where the Downloads folder is");
        }
        dir
    })
    .clone()
}
