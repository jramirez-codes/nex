//! The space after a glided (swiped) word, as Gboard types one in an ordinary text field.
//!
//! The on-screen keyboard can't put the space in itself: nex's input field is always empty
//! (see `android/java/NexInput.java`), so the keyboard never sees a word before the cursor to
//! space away from. [`AutoSpace`] follows what the keyboard types into the word it is
//! composing, and says when a word that ends should get a space after it: only a glided word,
//! never tapped characters.

/// The word the keyboard is composing, and whether it was glided.
#[derive(Debug, Default)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct AutoSpace {
    /// What the keyboard has typed of the word so far (after its takebacks).
    word: String,
    /// Whether a composing update put in more than one character at once: a glided word
    /// (tapping composes a character at a time).
    glided: bool,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
impl AutoSpace {
    pub const fn new() -> Self {
        Self { word: String::new(), glided: false }
    }

    /// The keyboard took back `delete` characters, then typed `text`; `composing` when this
    /// was the word it's still composing (rather than text it committed in one go: a tap, a
    /// suggestion, a pasted word).
    pub fn typed(&mut self, delete: usize, text: &str, composing: bool) {
        for _ in 0..delete {
            self.word.pop();
        }
        if composing && text.chars().count() > 1 {
            self.glided = true;
        }
        self.word.push_str(text);
    }

    /// The keyboard is done with the word: it `committed` it, or gave it up (a new input
    /// connection). Returns whether to type a space after it: a glided word that doesn't end
    /// in one already.
    pub fn word_done(&mut self, committed: bool) -> bool {
        let space = committed && self.glided && self.word.chars().last().is_some_and(|c| !c.is_whitespace());
        *self = Self::new();
        space
    }
}

#[cfg(test)]
mod tests {
    use super::AutoSpace;

    #[test]
    fn a_glided_word_gets_a_space() {
        let mut a = AutoSpace::default();
        a.typed(0, "hello", true);
        assert!(a.word_done(true));
    }

    #[test]
    fn a_glided_word_the_keyboard_revised_gets_one_space() {
        let mut a = AutoSpace::default();
        a.typed(0, "hells", true);
        a.typed(1, "o", true);
        // Committed as composed: nothing more to type.
        a.typed(0, "", false);
        assert!(a.word_done(true));
        // The next word starts afresh.
        assert!(!a.word_done(true));
    }

    #[test]
    fn tapped_characters_get_no_space() {
        let mut a = AutoSpace::default();
        a.typed(0, "l", false);
        assert!(!a.word_done(true));
        a.typed(0, "s", false);
        assert!(!a.word_done(true));
    }

    #[test]
    fn a_word_composed_by_taps_gets_no_space() {
        let mut a = AutoSpace::default();
        a.typed(0, "l", true);
        a.typed(0, "s", true);
        assert!(!a.word_done(true));
    }

    #[test]
    fn a_word_committed_in_one_go_gets_no_space() {
        // A suggestion or a pasted word: not glided.
        let mut a = AutoSpace::default();
        a.typed(0, "hello", false);
        assert!(!a.word_done(true));
    }

    #[test]
    fn a_glided_word_with_its_own_space_gets_no_other() {
        let mut a = AutoSpace::default();
        a.typed(0, "hello", true);
        a.typed(0, " ", false);
        assert!(!a.word_done(true));
    }

    #[test]
    fn a_glided_word_deleted_or_given_up_gets_no_space() {
        let mut a = AutoSpace::default();
        a.typed(0, "hello", true);
        a.typed(5, "", false);
        assert!(!a.word_done(true));

        a.typed(0, "hello", true);
        assert!(!a.word_done(false));
        // Given up: a tap after it isn't part of the glided word.
        a.typed(0, "x", false);
        assert!(!a.word_done(true));
    }
}
