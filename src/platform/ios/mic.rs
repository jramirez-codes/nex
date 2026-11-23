//! The microphone on iOS. Two things cpal leaves to the app:
//!
//! - **Permission.** iOS asks the user the first time an app asks for it; [`ask_mic`] asks,
//!   and nex shares the mic once [`mic_allowed`] (as on Android).
//! - **The audio session.** An app's session starts in a category that only plays, where iOS
//!   reports no input channels at all, so cpal finds no way to record. [`prepare_mic`] switches
//!   it to play-and-record before cpal opens the mic: mixed with other apps' audio, out of the
//!   speaker rather than the earpiece, and through Bluetooth headsets.
//!
//! The session's permission calls and `AllowBluetooth` are deprecated in favor of
//! `AVAudioApplication` (iOS 17) and `AllowBluetoothHFP` (iOS 26); the app supports iOS 15.
#![allow(deprecated)]

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryOptions, AVAudioSessionCategoryPlayAndRecord, AVAudioSessionRecordPermission};

use crate::platform::MicRefused;

fn permission() -> AVAudioSessionRecordPermission {
    // SAFETY: the shared session is a singleton, safe to query from any thread.
    unsafe { AVAudioSession::sharedInstance().recordPermission() }
}

/// Whether nex may start sharing the mic: not before the user has been asked. Once they've
/// answered, sharing starts, and a refusal shows up as [`MicRefused::Denied`].
pub fn mic_allowed() -> bool {
    permission() != AVAudioSessionRecordPermission::Undetermined
}

/// Ask the user to let nex use the microphone (iOS asks only once; after that it's in Settings).
/// Returns at once; [`mic_allowed`] is checked again when the mic is next needed.
pub fn ask_mic() {
    let answered = RcBlock::new(|_granted: Bool| {});
    // SAFETY: the shared session is a singleton; the block is copied by the call.
    unsafe {
        AVAudioSession::sharedInstance().requestRecordPermission(&answered)
    };
}

/// Get the audio session ready to record, just before the mic is opened.
pub fn prepare_mic() -> Result<(), MicRefused> {
    if permission() == AVAudioSessionRecordPermission::Denied {
        return Err(MicRefused::Denied);
    }
    // SAFETY: a UIKit constant, present on every iOS this app runs on.
    let Some(category) = (unsafe { AVAudioSessionCategoryPlayAndRecord }) else {
        return Err(MicRefused::Failed("iOS has no play-and-record audio category".into()));
    };
    let options = AVAudioSessionCategoryOptions::MixWithOthers
        | AVAudioSessionCategoryOptions::DefaultToSpeaker
        | AVAudioSessionCategoryOptions::AllowBluetooth;
    // SAFETY: the shared session is a singleton, safe to configure from any thread.
    unsafe {
        let session = AVAudioSession::sharedInstance();
        session.setCategory_withOptions_error(category, options).map_err(|e| MicRefused::Failed(format!("audio session: {e}")))?;
        session.setActive_error(true).map_err(|e| MicRefused::Failed(format!("audio session: {e}")))?;
    }
    Ok(())
}
