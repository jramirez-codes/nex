//! What changed in a text field, as keys: the characters to take back with Backspace, then
//! the text to type. For keyboards that edit a field of nex's own rather than sending keys
//! (iOS, see `ios/keyboard.rs`): a swiped word deleted whole, or a word the keyboard
//! rewrote, reaches the terminal as the Backspaces and typing that make the same change.

/// The edit that turns `old` into `new`: `(delete, text)`, keeping what they start with alike.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub fn edit_between(old: &str, new: &str) -> (usize, String) {
    let same = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
    let delete = old.chars().count() - same;
    (delete, new.chars().skip(same).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_adds_to_the_end() {
        assert_eq!(edit_between("ls", "ls -l"), (0, " -l".into()));
        assert_eq!(edit_between("", "hello "), (0, "hello ".into()));
    }

    #[test]
    fn deleting_takes_back() {
        assert_eq!(edit_between("git status", "git "), (6, String::new()));
        assert_eq!(edit_between("abc", ""), (3, String::new()));
    }

    #[test]
    fn a_rewritten_word_is_taken_back_and_retyped() {
        assert_eq!(edit_between("cd hte ", "cd the "), (4, "the ".into()));
        // Typed in the middle (the keyboard's cursor was moved): the rest is retyped after it.
        assert_eq!(edit_between("hello", "hexllo"), (3, "xllo".into()));
    }

    #[test]
    fn counts_characters_not_bytes() {
        assert_eq!(edit_between("café", "caf"), (1, String::new()));
        assert_eq!(edit_between("naïve", "naïve 👍"), (0, " 👍".into()));
    }

    #[test]
    fn nothing_changed() {
        assert_eq!(edit_between("same", "same"), (0, String::new()));
    }
}
