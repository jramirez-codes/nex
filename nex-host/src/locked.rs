//! Taking a lock even after a thread panicked while holding it.
//!
//! The daemon's agents, and each agent's screen, are shared by the threads of every agent and
//! every attached pane. Rust "poisons" a `Mutex` when a thread panics holding it, and
//! `lock().unwrap()` would then panic in every other thread too: one bug in one session would
//! take down the daemon and all its agents. A panic happens between two updates of the data,
//! and everything behind these locks stays usable after a half-done one (at worst a screen
//! misses a bit of output), so the others carry on instead.

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
}
