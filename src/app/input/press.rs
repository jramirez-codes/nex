//! How long a press is held before it means something else, shared by the touch controls:
//! the tab strip's long press (`chrome/titlebar.rs`) and the keyboard button's taps, joystick
//! and hold to talk (`chrome/keyboard_button.rs`). All in seconds.

/// Holding a tab this long before moving makes the drag reorder it instead of swiping the
/// strip.
pub(in crate::app) const LONG_PRESS: f64 = 0.35;
/// The longest gap between the taps of a double-tap (and so how long a tap waits).
pub(in crate::app) const DOUBLE_TAP: f64 = 0.3;
/// Holding the keyboard button's joystick: the first repeat comes after this long.
pub(in crate::app) const FIRST_REPEAT: f64 = 0.35;
/// A press held still this long talks (longer than [`DOUBLE_TAP`], so taps keep their meaning).
pub(in crate::app) const HOLD_TO_TALK: f64 = 0.45;
