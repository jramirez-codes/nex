//! A finger held still on a widget, to read what's cut off there: pane headers' titles and
//! the Agents pane's cards and rows (both scroll their long lines with [`marquee`]). egui's
//! own `long_touched` waits out its whole click time (0.8 s) first, which with the scroll's
//! opening pause felt slow; this takes a hold as soon as it can't be a tap.
//!
//! [`marquee`]: crate::ui::glide::marquee

use egui::Response;

/// A finger down this long without moving off a click is a hold, in seconds: as long as the
/// tab strip's long press (`app/input/press.rs`), well past a tap.
pub const HOLD: f64 = 0.35;

/// Whether a finger has been held still on `r` for [`HOLD`] or longer. True on every frame
/// from then until egui stops counting it as a possible click (it moved, or egui's own long
/// press took over); callers start their hold on the first.
pub fn held_still(r: &Response) -> bool {
    r.is_pointer_button_down_on()
        && r.ctx.input(|i| {
            i.any_touches()
                && i.pointer.primary_down()
                && i.pointer.could_any_button_be_click()
                && i.pointer.press_start_time().is_some_and(|t| i.time - t >= HOLD)
        })
}
