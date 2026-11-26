//! The system clipboard, through `UIPasteboard`. egui has no clipboard of its own on iOS,
//! only one inside the app, so nex reads and writes this one itself: text fields paste from
//! it, and whatever nex copies is mirrored to it.

use objc2_foundation::NSString;
use objc2_ui_kit::UIPasteboard;

/// The text on the system clipboard, if it holds any. (iOS shows its "pasted from" banner.)
pub fn clipboard_text() -> Option<String> {
    // SAFETY: reading the general pasteboard's string is safe from any thread.
    let text = unsafe { UIPasteboard::generalPasteboard().string() }?.to_string();
    (!text.is_empty()).then_some(text)
}

/// Mirror what nex copied this frame (egui's copy commands) to the system clipboard.
pub fn mirror_copies(ctx: &egui::Context) {
    let copied: Option<String> =
        ctx.output(|o| o.commands.iter().rev().find_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }));
    if let Some(text) = copied {
        // SAFETY: setting the general pasteboard's string is safe from any thread.
        unsafe { UIPasteboard::generalPasteboard().setString(Some(&NSString::from_str(&text))) };
    }
}
