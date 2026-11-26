//! Opening a link in Safari (or whichever app handles its scheme) through
//! `UIApplication.openURL:options:completionHandler:`, on the main thread as nex's frames are.

use objc2::MainThreadMarker;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSString, NSURL};
use objc2_ui_kit::UIApplication;

/// Hand `uri` to iOS. A string that isn't a URL, or one no app takes, is only logged.
pub fn open_uri(uri: &std::ffi::OsStr) {
    let uri = uri.to_string_lossy();
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(url) = NSURL::URLWithString(&NSString::from_str(&uri)) else {
        eprintln!("nex: {uri} isn't a link iOS can open");
        return;
    };
    let options = NSDictionary::<NSString, AnyObject>::new();
    // SAFETY: `options` is the empty dictionary of option keys to values the method expects.
    unsafe { UIApplication::sharedApplication(mtm).openURL_options_completionHandler(&url, &options, None) };
}
