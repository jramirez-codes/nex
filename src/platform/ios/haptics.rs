//! Haptic feedback through UIKit's feedback generators (main thread only, as nex's frames are).

use objc2::MainThreadMarker;
use objc2_ui_kit::{UIImpactFeedbackGenerator, UIImpactFeedbackStyle, UISelectionFeedbackGenerator};

use crate::platform::Haptic;

pub fn haptic(kind: Haptic) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    match kind {
        Haptic::Tick => UISelectionFeedbackGenerator::new(mtm).selectionChanged(),
        Haptic::Tap | Haptic::Confirm | Haptic::Release => {
            let style = match kind {
                Haptic::Confirm => UIImpactFeedbackStyle::Medium,
                Haptic::Release => UIImpactFeedbackStyle::Soft,
                _ => UIImpactFeedbackStyle::Light,
            };
            // `initWithStyle:view:` replaces it from iOS 17.5; this one still works everywhere.
            #[allow(deprecated)]
            UIImpactFeedbackGenerator::initWithStyle(mtm.alloc(), style).impactOccurred();
        }
    }
}
