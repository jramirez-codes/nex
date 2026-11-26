//! An agent pane's header title changing: the words that changed roll over, the old ones
//! rising a few pixels as they fade out while the new ones rise into place from below and
//! fade in, on the shared ease-out curve. The words both titles start with stay put, so
//! "Fixing tests" becoming "Fixing lints" only rolls "tests" over to "lints". With
//! animations off (`animations` in the config) the new title is simply there.
//!
//! The state only says what to draw where; the header paints it (see `workspace/draw.rs`),
//! and asks for the next frame only while a roll is under way.

use std::collections::HashMap;

use crate::layout::PaneId;
use crate::ui::glide::ease_out;

/// Length of a roll, in seconds.
pub(in crate::app) const ROLL: f64 = 0.25;
/// How far the words travel, in points: the old ones up by this much, the new ones up from
/// this far below.
pub(in crate::app) const RISE: f32 = 6.0;

/// One run of text in a frame of a roll: drawn `dy` points below where it rests, at `alpha`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct Layer<'a> {
    pub(in crate::app) text: &'a str,
    pub(in crate::app) dy: f32,
    pub(in crate::app) alpha: f32,
}

/// What a header draws this frame: the words that stay (`still`, at rest), then from where
/// they end, the new words and, while a roll is under way, the old ones on their way out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct RollFrame<'a> {
    pub(in crate::app) still: &'a str,
    pub(in crate::app) arriving: Layer<'a>,
    pub(in crate::app) leaving: Option<Layer<'a>>,
}

/// One header's title and the roll from the last one, if it's still under way.
#[derive(Default)]
pub(in crate::app) struct TitleRoll {
    /// The title shown (or arriving); `None` before the first.
    text: Option<String>,
    /// The title going away, while a roll is under way.
    leaving: Option<String>,
    /// How many bytes at the start both titles share and keep in place.
    keep: usize,
    /// When the roll started.
    start: f64,
}

impl TitleRoll {
    /// Show `title` from `now`: a new title rolls in over the last one, unless it's the first
    /// one or `animate` is false. The same title again changes nothing.
    pub(in crate::app) fn set(&mut self, now: f64, title: &str, animate: bool) {
        let Some(old) = self.text.as_deref() else {
            self.text = Some(title.to_owned());
            return;
        };
        if old == title {
            if !animate {
                self.leaving = None;
            }
            return;
        }
        if animate {
            self.keep = shared_words(old, title);
            self.leaving = self.text.replace(title.to_owned());
            self.start = now;
        } else {
            self.text = Some(title.to_owned());
            self.leaving = None;
        }
    }

    /// Still rolling at `now`: the header needs another frame.
    pub(in crate::app) fn moving(&self, now: f64) -> bool {
        self.leaving.is_some() && now - self.start < ROLL
    }

    /// What to draw at `now`; `None` once settled, when the title is drawn whole.
    pub(in crate::app) fn frame(&self, now: f64) -> Option<RollFrame<'_>> {
        let text = self.text.as_deref()?;
        let old = self.leaving.as_deref().filter(|_| self.moving(now))?;
        let e = ease_out(((now - self.start) / ROLL) as f32);
        Some(RollFrame {
            still: &text[..self.keep],
            arriving: Layer { text: &text[self.keep..], dy: RISE * (1.0 - e), alpha: e },
            leaving: Some(Layer { text: &old[self.keep..], dy: -RISE * e, alpha: 1.0 - e }).filter(|l| !l.text.is_empty()),
        })
    }
}

/// Every agent pane's [`TitleRoll`], by pane.
#[derive(Default)]
pub(in crate::app) struct TitleRolls(HashMap<PaneId, TitleRoll>);

impl TitleRolls {
    /// Pane `id`'s roll, after showing `title` from `now` (see [`TitleRoll::set`]).
    pub(in crate::app) fn show(&mut self, id: PaneId, now: f64, title: &str, animate: bool) -> &TitleRoll {
        let roll = self.0.entry(id).or_default();
        roll.set(now, title, animate);
        roll
    }

    /// Forget pane `id` (it isn't an agent pane anymore): its next title is a first one.
    pub(in crate::app) fn forget(&mut self, id: PaneId) {
        self.0.remove(&id);
    }

    /// Forget the panes `keep` says are gone.
    pub(in crate::app) fn retain(&mut self, mut keep: impl FnMut(PaneId) -> bool) {
        self.0.retain(|&id, _| keep(id));
    }
}

/// How many bytes at the start of `a` and `b` are the same whole words: the longest shared
/// start that ends, in both, at a word's edge (the end of the text, or next to a space).
/// Splitting only there keeps the still words and the rolling ones spaced as one line.
fn shared_words(a: &str, b: &str) -> usize {
    let edge = |s: &str, i: usize| i == 0 || i == s.len() || s[..i].ends_with(char::is_whitespace) || s[i..].starts_with(char::is_whitespace);
    let mut best = 0;
    let mut i = 0;
    for (ca, cb) in a.chars().zip(b.chars()) {
        if ca != cb {
            break;
        }
        i += ca.len_utf8();
        if edge(a, i) && edge(b, i) {
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_title_and_the_same_title_again_dont_roll() {
        let mut r = TitleRoll::default();
        r.set(1.0, "Fixing tests", true);
        assert!(!r.moving(1.0));
        assert_eq!(r.frame(1.0), None);
        r.set(2.0, "Fixing tests", true);
        assert!(!r.moving(2.0));
        assert_eq!(r.frame(2.0), None);
    }

    #[test]
    fn a_new_title_rises_in_as_the_old_one_rises_out() {
        let mut r = TitleRoll::default();
        r.set(0.0, "Reading files", true);
        r.set(10.0, "Running tests", true);
        assert!(r.moving(10.0));
        let start = r.frame(10.0).unwrap();
        assert_eq!(start.still, "");
        assert_eq!(start.arriving, Layer { text: "Running tests", dy: RISE, alpha: 0.0 });
        assert_eq!(start.leaving, Some(Layer { text: "Reading files", dy: 0.0, alpha: 1.0 }));

        // Halfway through in time it's well past halfway in motion: ease-out.
        let mid = r.frame(10.0 + ROLL / 2.0).unwrap();
        let e = ease_out(0.5);
        assert!(e > 0.5);
        assert_eq!(mid.arriving, Layer { text: "Running tests", dy: RISE * (1.0 - e), alpha: e });
        assert_eq!(mid.leaving, Some(Layer { text: "Reading files", dy: -RISE * e, alpha: 1.0 - e }));

        // Then it settles, and the header draws the title whole again.
        assert!(!r.moving(10.0 + ROLL));
        assert_eq!(r.frame(10.0 + ROLL), None);
    }

    #[test]
    fn only_the_words_that_changed_roll() {
        let mut r = TitleRoll::default();
        r.set(0.0, "Fixing the tests", true);
        r.set(1.0, "Fixing the lints", true);
        let f = r.frame(1.0).unwrap();
        assert_eq!(f.still, "Fixing the ");
        assert_eq!(f.arriving.text, "lints");
        assert_eq!(f.leaving.unwrap().text, "tests");

        // Words added at the end: only they come in, nothing leaves.
        r.set(5.0, "Fixing the lints again", true);
        let f = r.frame(5.0).unwrap();
        assert_eq!(f.still, "Fixing the lints");
        assert_eq!(f.arriving.text, " again");
        assert_eq!(f.leaving, None);
    }

    #[test]
    fn a_word_is_never_split() {
        assert_eq!(shared_words("Testing", "Tests"), 0);
        assert_eq!(shared_words("Run tests", "Run testing"), 4);
        assert_eq!(shared_words("Run", "Run it"), 3);
        assert_eq!(shared_words("Run it", "Run"), 3);
        assert_eq!(shared_words("same", "same"), 4);
        assert_eq!(shared_words("", "new"), 0);
        // Multi-byte text splits on characters, not bytes.
        assert_eq!(shared_words("Prüfe die Tests", "Prüfe den Code"), "Prüfe ".len());
    }

    #[test]
    fn a_change_mid_roll_rolls_on_from_the_title_now_shown() {
        let mut r = TitleRoll::default();
        r.set(0.0, "One", true);
        r.set(1.0, "Two", true);
        r.set(1.1, "Three", true);
        let f = r.frame(1.1).unwrap();
        assert_eq!(f.arriving.text, "Three");
        assert_eq!(f.leaving.unwrap().text, "Two");
        assert!(r.moving(1.1 + ROLL - 0.01));
        assert!(!r.moving(1.1 + ROLL));
    }

    #[test]
    fn with_animations_off_a_new_title_is_simply_there() {
        let mut r = TitleRoll::default();
        r.set(0.0, "Reading files", false);
        r.set(1.0, "Running tests", false);
        assert!(!r.moving(1.0));
        assert_eq!(r.frame(1.0), None);

        // Turned off mid-roll, the roll stops.
        r.set(2.0, "Writing code", true);
        assert!(r.moving(2.0));
        r.set(2.05, "Writing code", false);
        assert!(!r.moving(2.05));
        assert_eq!(r.frame(2.05), None);
    }

    #[test]
    fn rolls_are_kept_by_pane_and_forgotten() {
        let mut rolls = TitleRolls::default();
        rolls.show(1, 0.0, "A", true);
        rolls.show(2, 0.0, "B", true);
        assert!(rolls.show(1, 1.0, "A2", true).moving(1.0));
        assert!(!rolls.show(2, 1.0, "B", true).moving(1.0));
        rolls.forget(1);
        assert!(!rolls.show(1, 2.0, "A3", true).moving(2.0), "a forgotten pane's next title is a first one");
        rolls.retain(|id| id != 2);
        assert!(!rolls.show(2, 3.0, "B2", true).moving(3.0));
    }
}
