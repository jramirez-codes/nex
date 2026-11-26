//! Reading a pane header's title that's too long to fit: while a finger is held on the
//! header (or the secondary mouse button, on desktop), the title scrolls sideways,
//! marquee-style (the timing the Agents pane's held cards use, `ui::glide::marquee`): a
//! blink's pause, along to its end, a pause there, and back, for as long as the hold lasts.
//! Letting go snaps it back to the usual faded-out start. A title that fits doesn't move;
//! with animations off a held title shows its end.
//!
//! The state only says how far along the title is; the header paints it (see
//! `workspace/draw.rs`), and asks for the next frame while a hold lasts. Only one header can
//! be held at a time, so there is one hold, not one per pane.

use crate::layout::PaneId;
use crate::ui::glide::marquee;

/// The header being held, if one is.
#[derive(Default)]
pub(in crate::app) struct TitleScroll {
    hold: Option<Hold>,
}

struct Hold {
    pane: PaneId,
    /// When the hold started.
    since: f64,
    /// How far the title was drawn scrolled, the last frame it was drawn.
    offset: f32,
}

impl TitleScroll {
    /// Pane `id`'s header is held from `now`. Holding it still keeps the scroll going.
    pub(in crate::app) fn hold(&mut self, id: PaneId, now: f64) {
        if self.hold.as_ref().is_none_or(|h| h.pane != id) {
            self.hold = Some(Hold { pane: id, since: now, offset: 0.0 });
        }
    }

    /// The hold is over: the title goes back to its start.
    pub(in crate::app) fn release(&mut self) {
        self.hold = None;
    }

    /// Whether pane `id`'s header is held.
    pub(in crate::app) fn held(&self, id: PaneId) -> bool {
        self.hold.as_ref().is_some_and(|h| h.pane == id)
    }

    /// How far to scroll pane `id`'s title at `now`, if it is held: `overflow` is how much
    /// wider the title is than its room; with animations off (`animate`) its end shows.
    /// `None` when it isn't held.
    pub(in crate::app) fn offset(&mut self, id: PaneId, now: f64, overflow: f32, animate: bool) -> Option<f32> {
        let h = self.hold.as_mut().filter(|h| h.pane == id)?;
        h.offset = marquee(now - h.since, overflow, animate);
        Some(h.offset)
    }

    /// How far the held title was last drawn scrolled, and in which pane (for tests).
    #[cfg(test)]
    pub(in crate::app) fn shown(&self) -> Option<(PaneId, f32)> {
        self.hold.as_ref().map(|h| (h.pane, h.offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_another_header_starts_over_and_letting_go_ends_it() {
        let (a, b) = (1, 2);
        let mut s = TitleScroll::default();
        assert_eq!(s.offset(a, 0.0, 50.0, true), None);
        s.hold(a, 0.0);
        // Under way a moment after the hold takes.
        assert!(s.offset(a, 0.15, 50.0, true).unwrap() > 0.0);
        // Holding on doesn't restart it.
        s.hold(a, 1.5);
        let x = s.offset(a, 1.0, 50.0, true).unwrap();
        assert!(x > 0.0);
        assert_eq!(s.offset(b, 1.0, 50.0, true), None, "only the held header scrolls");
        s.hold(b, 3.0);
        assert!(!s.held(a) && s.held(b));
        assert_eq!(s.offset(b, 3.0, 50.0, true), Some(0.0));
        s.release();
        assert!(!s.held(b));
        assert_eq!(s.offset(b, 4.0, 50.0, true), None);
    }
}
