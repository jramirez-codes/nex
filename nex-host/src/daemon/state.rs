//! What the daemon holds, shared by every connection and its background threads: the
//! agents, the post office, the messages waiting to be typed in, and the counters. Its fields
//! are private: the rest of the daemon goes through the methods here, so what happens under
//! which lock is decided in this one file.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nex_host::proto;

use crate::daemon::agent::Agent;
use crate::daemon::broker::{Broker, Peer};
use crate::util::{Locked, now_ms};

pub(super) struct State {
    agents: Mutex<HashMap<String, Arc<Agent>>>,
    clients: AtomicUsize,
    next_token: AtomicU64,
    broker: Broker,
    /// Messages to type in once their recipient can take them: started up, and not asking
    /// the user something.
    pending: Mutex<Vec<(String, String)>>,
    /// A newer nex-host is installed and this one is on its way out (see `retire.rs`): set
    /// under the `agents` lock, after which nothing is started.
    retiring: AtomicBool,
}

impl State {
    /// An empty daemon, its post office recording mailboxes on disk.
    pub(super) fn new() -> Self {
        Self::with_broker(Broker::new())
    }

    fn with_broker(broker: Broker) -> Self {
        Self {
            agents: Mutex::new(HashMap::new()),
            clients: AtomicUsize::new(0),
            next_token: AtomicU64::new(1),
            broker,
            pending: Mutex::new(Vec::new()),
            retiring: AtomicBool::new(false),
        }
    }

    /// An empty daemon for tests: nothing recorded on disk.
    #[cfg(test)]
    pub(super) fn in_memory() -> Self {
        Self::with_broker(Broker::default())
    }

    pub(super) fn broker(&self) -> &Broker {
        &self.broker
    }

    // ---- agents ----

    pub(super) fn get(&self, id: &str) -> Option<Arc<Agent>> {
        self.agents.locked().get(id).cloned()
    }

    pub(super) fn contains(&self, id: &str) -> bool {
        self.agents.locked().contains_key(id)
    }

    /// The agents for which `keep` holds, exited ones included, in no particular order.
    pub(super) fn agents_where(&self, keep: impl Fn(&Agent) -> bool) -> Vec<Arc<Agent>> {
        self.agents.locked().values().filter(|a| keep(a)).cloned().collect()
    }

    pub(super) fn has_agents(&self) -> bool {
        !self.agents.locked().is_empty()
    }

    pub(super) fn peers(&self) -> Vec<Peer> {
        self.agents.locked().values().map(|a| Peer { id: a.id.clone(), name: a.name.clone(), parent: a.parent.clone(), exited: a.exited() }).collect()
    }

    pub(super) fn sessions(&self) -> Vec<proto::Session> {
        let now = now_ms();
        let mut list: Vec<_> = self.agents.locked().values().map(|a| a.session(now)).collect();
        list.sort_by_key(|s| s.started);
        list
    }

    /// Start an agent as `id` with `start`, unless the daemon is retiring. The agents stay
    /// locked from before it starts until it's on the list: what it runs first (`nex-host
    /// send`, `done`) must find it there, or it's taken for someone at a shell.
    pub(super) fn start(&self, id: String, start: impl FnOnce() -> io::Result<Arc<Agent>>) -> io::Result<()> {
        let mut agents = self.agents.locked();
        if self.retiring() {
            return Err(io::Error::other(proto::RESTARTING));
        }
        let a = start()?;
        agents.insert(id, a);
        Ok(())
    }

    /// Drop a session, and its mail with it.
    pub(super) fn remove(&self, id: &str) {
        self.agents.locked().remove(id);
        self.broker.forget(id);
    }

    // ---- messages waiting to be typed in ----

    /// Type `text` into agent `id` once it can take it.
    pub(super) fn queue(&self, id: &str, text: String) {
        self.pending.locked().push((id.to_string(), text));
    }

    /// Everything waiting to be typed in, oldest first, leaving nothing behind.
    pub(super) fn take_pending(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.pending.locked())
    }

    /// Put back what [`State::take_pending`] took and couldn't type in yet, ahead of anything
    /// queued meanwhile.
    pub(super) fn requeue(&self, still: Vec<(String, String)>) {
        self.pending.locked().splice(0..0, still);
    }

    // ---- connections and tokens ----

    pub(super) fn connected(&self) {
        self.clients.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn disconnected(&self) {
        self.clients.fetch_sub(1, Ordering::Relaxed);
    }

    pub(super) fn clients(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }

    /// A number no attachment has had (and, for ids, some noise).
    pub(super) fn next_token(&self) -> u64 {
        self.next_token.fetch_add(1, Ordering::Relaxed)
    }

    // ---- retiring ----

    pub(super) fn retiring(&self) -> bool {
        self.retiring.load(Ordering::Relaxed)
    }

    /// Whether nothing runs: no agent (one that exited doesn't count) and no `recv` waiting.
    pub(super) fn idle(&self) -> bool {
        Self::idle_in(&self.agents.locked(), &self.broker)
    }

    fn idle_in(agents: &HashMap<String, Arc<Agent>>, broker: &Broker) -> bool {
        !agents.values().any(|a| !a.exited()) && !broker.waiting()
    }

    /// Start retiring if nothing runs, under the agents lock that [`State::start`] takes too:
    /// once this is set, nothing starts. Whether it did.
    pub(super) fn retire_if_idle(&self) -> bool {
        let agents = self.agents.locked();
        if !Self::idle_in(&agents, &self.broker) {
            return false;
        }
        self.retiring.store(true, Ordering::Relaxed);
        true
    }

    /// Retiring from the start, for tests.
    #[cfg(test)]
    pub(super) fn set_retiring(&self) {
        self.retiring.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_could_not_be_typed_in_goes_back_ahead_of_newer() {
        let state = State::in_memory();
        state.queue("a", "one".into());
        state.queue("b", "two".into());
        let taken = state.take_pending();
        assert_eq!(taken, [("a".to_string(), "one".to_string()), ("b".into(), "two".into())]);
        assert!(state.take_pending().is_empty(), "taking leaves nothing behind");
        state.queue("c", "three".into());
        state.requeue(vec![taken[1].clone()]);
        assert_eq!(state.take_pending(), [("b".to_string(), "two".to_string()), ("c".into(), "three".into())]);
    }

    #[test]
    fn an_empty_daemon_is_idle_and_retires() {
        let state = State::in_memory();
        assert!(state.idle() && !state.retiring());
        assert!(state.retire_if_idle());
        assert!(state.retiring());
        let e = state.start("a1".into(), || unreachable!("nothing starts once retiring")).unwrap_err();
        assert_eq!(e.to_string(), proto::RESTARTING);
        assert!(!state.has_agents());
    }

    #[test]
    fn tokens_are_never_reused() {
        let state = State::in_memory();
        let (a, b) = (state.next_token(), state.next_token());
        assert_ne!(a, b);
    }
}
