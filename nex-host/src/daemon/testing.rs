//! What the daemon's tests share: an in-memory [`State`] (nothing recorded on disk), and
//! agents that are plain programs (`sleep`, `sh`) rather than agent CLIs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::daemon::agent::{Agent, Spec};
use crate::daemon::lifecycle::launch;
use crate::daemon::state::State;

pub(super) fn state() -> Arc<State> {
    Arc::new(State::in_memory())
}

/// Run directly in this environment, not under the login shell.
pub(super) fn env() -> Option<Vec<(String, String)>> {
    Some(vec![("PATH".into(), std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()))])
}

/// Start `argv` as agent `id` called `name`, of a kind no agent CLI has.
pub(super) fn start(state: &Arc<State>, id: &str, name: &str, argv: &[&str], parent: Option<&str>) -> Arc<Agent> {
    start_kind(state, id, name, argv, parent, "plain")
}

pub(super) fn start_kind(state: &Arc<State>, id: &str, name: &str, argv: &[&str], parent: Option<&str>, kind: &str) -> Arc<Agent> {
    let spec = Spec {
        argv: argv.iter().map(|a| a.to_string()).collect(),
        kind: Some(kind.into()),
        name: Some(name.into()),
        cwd: "/".into(),
        cols: 80,
        rows: 24,
        env: env(),
        parent: parent.map(str::to_string),
        role: None,
    };
    launch(state, id.into(), spec).expect("start a plain program");
    state.get(id).expect("it's on the list")
}

/// Wait up to five seconds for `done`.
pub(super) fn wait_for(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// End every agent still running, so no `sleep` outlives the test.
pub(super) fn kill_all(state: &State) {
    for a in state.agents_where(|a| !a.exited()) {
        a.kill();
    }
}
