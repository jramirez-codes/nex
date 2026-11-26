//! Background jobs: work that runs on its own thread and hands its result back to the frame
//! loop, which picks it up without waiting (see "Background work never blocks a frame" in
//! ARCHITECTURE.md).
//!
//! - [`Job`] is one job at a time (a folder listing, a git status, a login). Starting another
//!   supersedes the one running: whatever that one still sends is dropped, so a slow answer
//!   to an old question never replaces a newer one. [`Job::stream`] is the same for work that
//!   sends several messages (a diff, then its syntax colors a chunk at a time).
//! - [`Jobs`] is many at once (file copies, uploads), each with what the caller keeps
//!   about it (its label, its progress) until it finishes.
//!
//! Each job wakes the UI ([`Wake`]) once its result is sent, so the frame that polls it comes
//! right away.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, OnceLock};

/// What a job wakes once it has sent something: usually the egui context to repaint.
pub trait Wake: Send + 'static {
    fn wake(&self);
}

impl Wake for egui::Context {
    fn wake(&self) {
        self.request_repaint();
    }
}

/// A context that may not be known yet when the job starts (looked up after sending: a paint
/// that sets it later also polls later).
impl Wake for Arc<OnceLock<egui::Context>> {
    fn wake(&self) {
        if let Some(ctx) = self.get() {
            ctx.request_repaint();
        }
    }
}

/// How a [`Job::stream`] sends its messages.
pub struct Sender<T> {
    tx: mpsc::Sender<(u64, T)>,
    generation: u64,
    wake: Box<dyn Wake>,
}

impl<T> Sender<T> {
    /// Send `msg` and wake the UI. False once nobody listens (the job was superseded or
    /// cancelled): the work can stop early.
    pub fn send(&self, msg: T) -> bool {
        let ok = self.tx.send((self.generation, msg)).is_ok();
        self.wake.wake();
        ok
    }
}

/// One background job at a time; see the module docs.
pub struct Job<T> {
    rx: Option<Receiver<(u64, T)>>,
    /// Bumped by every start and cancel: only messages from the current one are taken.
    generation: u64,
    /// The job sends one result and is done with it (else it streams until it hangs up).
    once: bool,
}

impl<T> Default for Job<T> {
    fn default() -> Self {
        Job { rx: None, generation: 0, once: true }
    }
}

impl<T: Send + 'static> Job<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `work` on a thread named `name`, superseding the job running. False if the thread
    /// couldn't be started (nothing is running then).
    pub fn start(&mut self, name: &str, wake: impl Wake, work: impl FnOnce() -> T + Send + 'static) -> bool {
        self.spawn(name, wake, true, move |tx| {
            tx.send(work());
        })
    }

    /// [`start`](Self::start) for work that sends any number of messages; it is done when
    /// `work` returns.
    pub fn stream(&mut self, name: &str, wake: impl Wake, work: impl FnOnce(&Sender<T>) + Send + 'static) -> bool {
        self.spawn(name, wake, false, work)
    }

    fn spawn(&mut self, name: &str, wake: impl Wake, once: bool, work: impl FnOnce(&Sender<T>) + Send + 'static) -> bool {
        self.generation += 1;
        self.once = once;
        let (tx, rx) = mpsc::channel();
        let tx = Sender { tx, generation: self.generation, wake: Box::new(wake) };
        let spawned = std::thread::Builder::new().name(name.into()).spawn(move || work(&tx)).is_ok();
        self.rx = spawned.then_some(rx);
        spawned
    }

    /// The next message from the current job, if one is in. A job that sends one result is
    /// over once it's taken; any job is over once its thread ends.
    pub fn poll(&mut self) -> Option<T> {
        loop {
            let rx = self.rx.as_ref()?;
            match rx.try_recv() {
                Ok((generation, msg)) if generation == self.generation => {
                    if self.once {
                        self.rx = None;
                    }
                    return Some(msg);
                }
                Ok(_) => continue,
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    return None;
                }
            }
        }
    }

    /// A job is on its way (started, and its result not yet taken).
    pub fn running(&self) -> bool {
        self.rx.is_some()
    }

    /// Forget the job running: whatever it still sends is dropped.
    pub fn cancel(&mut self) {
        self.generation += 1;
        self.rx = None;
    }
}

/// Many background jobs at once, each with `M`, what the caller keeps about it.
pub struct Jobs<T, M = ()> {
    running: Vec<(M, Receiver<T>)>,
}

impl<T, M> Default for Jobs<T, M> {
    fn default() -> Self {
        Jobs { running: Vec::new() }
    }
}

impl<T: Send + 'static, M> Jobs<T, M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `work` on a thread named `name`, beside the jobs already running. If the thread
    /// couldn't be started, the next [`poll`](Self::poll) hands `meta` back without a result.
    pub fn push(&mut self, name: &str, wake: impl Wake, meta: M, work: impl FnOnce() -> T + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
            let _ = tx.send(work());
            wake.wake();
        });
        self.running.push((meta, rx));
    }

    /// The jobs that finished since the last poll, in the order they were started: each
    /// one's `M` and its result (`None`: its thread ended without one).
    pub fn poll(&mut self) -> Vec<(M, Option<T>)> {
        let mut finished = Vec::new();
        let mut i = 0;
        while i < self.running.len() {
            let result = match self.running[i].1.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => {
                    i += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => None,
            };
            finished.push((self.running.remove(i).0, result));
        }
        finished
    }

    /// What is kept about each job still running, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &M> {
        self.running.iter().map(|(m, _)| m)
    }

    pub fn first(&self) -> Option<&M> {
        self.iter().next()
    }

    pub fn len(&self) -> usize {
        self.running.len()
    }

    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// Counts its wakes.
    #[derive(Clone, Default)]
    struct Counter(Arc<AtomicUsize>);

    impl Wake for Counter {
        fn wake(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Counter {
        fn count(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    /// Poll until `f` gives something, as frames would.
    fn wait<R>(mut f: impl FnMut() -> Option<R>) -> R {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(r) = f() {
                return r;
            }
            assert!(Instant::now() < deadline, "the job never finished");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_job_delivers_its_result_once_and_wakes() {
        let wakes = Counter::default();
        let mut job = Job::new();
        assert!(!job.running());
        assert!(job.start("test", wakes.clone(), || 42));
        assert!(job.running());
        assert_eq!(wait(|| job.poll()), 42);
        assert!(!job.running(), "taking the result ends the job");
        assert_eq!(job.poll(), None);
        assert_eq!(wakes.count(), 1);
    }

    #[test]
    fn starting_again_drops_the_superseded_result() {
        let (go, wait_go) = mpsc::channel::<()>();
        let mut job = Job::new();
        job.start("old", Counter::default(), move || {
            let _ = wait_go.recv();
            "old"
        });
        job.start("new", Counter::default(), || "new");
        go.send(()).unwrap();
        assert_eq!(wait(|| job.poll()), "new");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(job.poll(), None);
    }

    #[test]
    fn a_cancelled_job_delivers_nothing() {
        let (go, wait_go) = mpsc::channel::<()>();
        let mut job = Job::new();
        job.start("test", Counter::default(), move || {
            let _ = wait_go.recv();
            1
        });
        job.cancel();
        assert!(!job.running());
        let _ = go.send(());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(job.poll(), None);
    }

    #[test]
    fn a_stream_delivers_in_order_until_its_work_returns() {
        let wakes = Counter::default();
        let mut job = Job::new();
        job.stream("test", wakes.clone(), |tx| {
            for i in 0..3 {
                assert!(tx.send(i));
            }
        });
        let got: Vec<i32> = (0..3).map(|_| wait(|| job.poll())).collect();
        assert_eq!(got, [0, 1, 2]);
        wait(|| {
            job.poll();
            (!job.running()).then_some(())
        });
        assert!(!job.running(), "the stream ends when its thread does");
        assert_eq!(wakes.count(), 3);
    }

    #[test]
    fn a_superseded_stream_is_told_to_stop() {
        let (go, wait_go) = mpsc::channel::<()>();
        let (said, heard) = mpsc::channel::<bool>();
        let mut job = Job::new();
        job.stream("test", Counter::default(), move |tx| {
            let _ = wait_go.recv();
            let _ = said.send(tx.send(1));
        });
        job.cancel();
        go.send(()).unwrap();
        assert!(!heard.recv().unwrap(), "nobody listens any more");
    }

    #[test]
    fn many_jobs_finish_in_any_order_with_what_was_kept() {
        let wakes = Counter::default();
        let (go, wait_go) = mpsc::channel::<()>();
        let mut jobs: Jobs<u32, &str> = Jobs::new();
        jobs.push("slow", wakes.clone(), "slow", move || {
            let _ = wait_go.recv();
            1
        });
        jobs.push("quick", wakes.clone(), "quick", || 2);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs.first(), Some(&"slow"));
        let done = wait(|| Some(jobs.poll()).filter(|d| !d.is_empty()));
        assert_eq!(done, [("quick", Some(2))]);
        assert_eq!(jobs.iter().collect::<Vec<_>>(), [&"slow"]);
        go.send(()).unwrap();
        let done = wait(|| Some(jobs.poll()).filter(|d| !d.is_empty()));
        assert_eq!(done, [("slow", Some(1))]);
        assert!(jobs.is_empty());
        assert_eq!(wakes.count(), 2);
    }

    #[test]
    fn a_job_that_dies_is_handed_back_without_a_result() {
        let mut jobs: Jobs<u32, &str> = Jobs::new();
        jobs.push("dies", Counter::default(), "dies", || panic!("on purpose"));
        let done = wait(|| Some(jobs.poll()).filter(|d| !d.is_empty()));
        assert_eq!(done, [("dies", None)]);
    }
}
