//! Press and hold a card or a helper row to read what's cut off: while the finger stays
//! down, each of its lines too long for its room scrolls along, marquee-style, to its end and
//! back again (pausing at each end), and lifting the finger snaps it back to the faded,
//! cut-off line. The hold never opens the agent, and the list doesn't scroll under it. A
//! right click does the same on a desktop, until the next press or the pointer leaves.
//! Lines that fit don't move. With animations off a held line shows its end, still.
//!
//! The state is here ([`Reveal`]) with the scrolling's timing ([`offset`], shared with pane headers' titles
//! from `ui::glide`); the painters
//! (`paint`) draw each line [`offset`] along.

use crate::layout::PaneId;

/// What's held: a card (a team's header) or a helper's row in an open team.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Held {
    Card(PaneId),
    Row(PaneId),
}

/// A press and hold under way (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Reveal {
    pub(super) held: Held,
    /// When it started.
    pub(super) since: f64,
    /// Held by a finger (it lasts until it lifts), or a right click's (until the next press,
    /// or the pointer leaving it).
    pub(super) touch: bool,
    /// How far along its lines were drawn last frame (the most of them), in points: 0 while
    /// they all fit.
    pub(super) shifted: f32,
    /// Whether what it holds was drawn this frame (and, a right click's, under the pointer):
    /// once it isn't, it's over.
    pub(super) drawn: bool,
}

impl Reveal {
    pub(super) fn new(held: Held, since: f64, touch: bool) -> Reveal {
        Reveal { held, since, touch, shifted: 0.0, drawn: false }
    }

    /// Whether it's over as a frame begins: a finger's once no button is `down` any more, a
    /// right click's at the next press (`pressed`). (Either is also over once what it holds
    /// isn't drawn, and a right click's once the pointer is off it: see `drawn`.)
    pub(super) fn ended(&self, down: bool, pressed: bool) -> bool {
        if self.touch { !down } else { pressed }
    }
}

/// How far along to draw a line `overflow` points too long for its room, `held` seconds into
/// a hold: the marquee timing pane headers' titles share (see [`marquee`](crate::ui::glide::marquee)).
pub(super) use crate::ui::glide::marquee as offset;
