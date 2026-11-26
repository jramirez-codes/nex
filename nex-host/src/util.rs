//! Helpers both sides of nex-host use: the daemon and the commands that talk to it.
//!
//! Taking a lock even after a thread panicked while holding it ([`Locked`]): the daemon's
//! agents, and each agent's screen, are shared by the threads of every agent and every
//! attached pane. Rust "poisons" a `Mutex` when a thread panics holding it, and
//! `lock().unwrap()` would then panic in every other thread too: one bug in one session would
//! take down the daemon and all its agents. A panic happens between two updates of the data,
//! and everything behind these locks stays usable after a half-done one (at worst a screen
//! misses a bit of output), so the others carry on instead.
//!
//! The clock ([`now_ms`]) is the library's (`nex_host::subagents`), re-exported so both share one.

use std::sync::{Mutex, MutexGuard, PoisonError};

pub trait Locked<T> {
    /// `lock()`, ignoring poisoning.
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub use nex_host::subagents::now_ms;

/// The first line of `body` with text on it, trimmed, however long.
pub fn first_text_line(body: &str) -> &str {
    body.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
}

/// [`first_text_line`], at most 100 characters: what a reply quotes.
pub fn first_line(body: &str) -> String {
    let line = first_text_line(body);
    let mut out: String = line.chars().take(100).collect();
    if out.len() < line.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_panic_holding_the_lock_leaves_it_usable() {
        let m = Arc::new(Mutex::new(vec![1]));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let mut v = m2.lock().unwrap();
            v.push(2);
            panic!("a bug in one session");
        })
        .join();
        assert!(m.is_poisoned());
        m.locked().push(3);
        assert_eq!(*m.locked(), [1, 2, 3]);
    }

    #[test]
    fn first_lines() {
        assert_eq!(first_line("\n  Review the diff\nmore"), "Review the diff");
        assert!(first_line(&"x".repeat(300)).ends_with('…'));
        assert_eq!(first_text_line(&"x".repeat(300)).len(), 300, "uncut");
    }
}
