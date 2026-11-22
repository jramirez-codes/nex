//! The directory a shell reports with OSC 7 (`ESC ] 7 ; file://host/path BEL`).
//!
//! alacritty_terminal ignores OSC 7, so the PTY's output is scanned on its way to the parser.
//! nex reads a local shell's directory from the OS, so this matters for `ssh` sessions: a
//! remote shell that reports its directory lets a split or new tab open in that same place
//! on the remote host (see [`crate::remote::SshTarget::session_command`]).
//!
//! The same scan also spots desktop-notification OSCs (9, 99 and 777), which programs such as
//! Claude Code send when they want the user's attention; see [`Alert`].
//!
//! Only Unix wraps the PTY with [`ScanPty`]; the scanner itself is platform-neutral (and
//! tested everywhere), so it is unused, not broken, on Windows.
#![cfg_attr(not(unix), allow(dead_code))]

use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// A directory reported with OSC 7.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The host the shell runs on (empty when the shell left it out).
    pub host: String,
    pub path: String,
}

/// The latest report, shared between the PTY thread and the pane.
pub type Slot = Arc<Mutex<Option<Report>>>;

/// The `nex-host` agent the pane is showing, as `nex-host attach` announces it (see
/// `nex_host::proto::AGENT_TAG`), shared between the PTY thread and the pane.
pub type AgentSlot = Arc<Mutex<Option<String>>>;

/// Raised by the PTY thread when the program sends a notification OSC; the pane takes it on
/// its next frame. Raising it repaints the window even when the pane is hidden.
#[derive(Clone, Default)]
pub struct Alert {
    raised: Arc<AtomicBool>,
    repaint: Arc<OnceLock<egui::Context>>,
}

impl Alert {
    pub fn new(repaint: Arc<OnceLock<egui::Context>>) -> Self {
        Self { raised: Arc::default(), repaint }
    }

    fn raise(&self) {
        self.raised.store(true, Ordering::Relaxed);
        if let Some(ctx) = self.repaint.get() {
            ctx.request_repaint();
        }
    }

    /// Whether a notification came in since the last call.
    pub fn take(&self) -> bool {
        self.raised.swap(false, Ordering::Relaxed)
    }
}

/// Longest OSC body kept; longer ones (images, say) are skipped.
const MAX_OSC: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    /// Just saw `ESC`.
    Esc,
    /// Inside `ESC ]`, collecting the body.
    Osc,
    /// Saw `ESC` inside an OSC: `\` ends it.
    OscEsc,
    /// Inside an OSC too long to keep: wait for its end.
    Skip,
    SkipEsc,
}

/// Finds OSC 7 reports in a byte stream that may be split anywhere.
pub struct Scanner {
    state: State,
    buf: Vec<u8>,
    /// Set when a notification OSC completes; cleared by whoever reads it.
    pub notified: bool,
    /// Set when `nex-host attach` announces an agent (`Some("")`: it stopped showing one);
    /// taken by whoever reads it.
    pub agent: Option<String>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self { state: State::Ground, buf: Vec::new(), notified: false, agent: None }
    }
}

impl Scanner {
    /// Scan the next chunk of output; returns the last report it completed, if any.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<Report> {
        let mut found = None;
        let mut i = 0;
        while i < bytes.len() {
            if self.state == State::Ground {
                // Most output has no escapes worth a look: jump to the next one.
                match bytes[i..].iter().position(|&b| b == 0x1b) {
                    Some(at) => {
                        i += at + 1;
                        self.state = State::Esc;
                    }
                    None => break,
                }
                continue;
            }
            let b = bytes[i];
            i += 1;
            self.state = match (self.state, b) {
                (State::Esc, b']') => {
                    self.buf.clear();
                    State::Osc
                }
                (State::Esc, 0x1b) => State::Esc,
                (State::Osc, 0x07) => {
                    found = self.finish().or(found);
                    State::Ground
                }
                (State::Osc, 0x1b) => State::OscEsc,
                (State::Osc, _) if self.buf.len() >= MAX_OSC => State::Skip,
                (State::Osc, b) => {
                    self.buf.push(b);
                    State::Osc
                }
                (State::OscEsc, b'\\') => {
                    found = self.finish().or(found);
                    State::Ground
                }
                // Another escape cuts the OSC short; `ESC ESC` starts over.
                (State::OscEsc, b']') => {
                    self.buf.clear();
                    State::Osc
                }
                (State::OscEsc | State::SkipEsc, 0x1b) => State::Esc,
                (State::Skip, 0x07) => State::Ground,
                (State::Skip, 0x1b) => State::SkipEsc,
                (State::Skip, _) => State::Skip,
                (State::SkipEsc, b'\\') => State::Ground,
                (State::SkipEsc, _) => State::Skip,
                _ => State::Ground,
            };
        }
        found
    }

    fn finish(&mut self) -> Option<Report> {
        if is_notification(&self.buf) {
            self.notified = true;
            return None;
        }
        if let Some(id) = self.buf.strip_prefix(nex_host::proto::AGENT_TAG.as_bytes()) {
            let id = String::from_utf8_lossy(id);
            // Session ids are short hex; anything else isn't one of ours.
            if id.len() <= 32 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
                self.agent = Some(id.into_owned());
            }
            return None;
        }
        let body = self.buf.strip_prefix(b"7;")?;
        parse(&String::from_utf8_lossy(body))
    }
}

/// A desktop notification: OSC 9 (iTerm2), OSC 777 `notify` (urxvt, Ghostty) or OSC 99
/// (kitty). ConEmu's numbered OSC 9 commands (`9;4;…` is a progress bar) and kitty's
/// capability queries are not notifications.
fn is_notification(body: &[u8]) -> bool {
    if let Some(text) = body.strip_prefix(b"9;") {
        let digits = text.iter().take_while(|b| b.is_ascii_digit()).count();
        let numbered = digits > 0 && matches!(text.get(digits), None | Some(b';'));
        return !numbered;
    }
    if let Some(rest) = body.strip_prefix(b"99;") {
        let meta = rest.split(|&b| b == b';').next().unwrap_or_default();
        return !meta.windows(3).any(|w| w == b"p=?");
    }
    body.starts_with(b"777;notify;")
}

/// `file://host/path` (or another scheme, such as kitty's `kitty-shell-cwd://`).
fn parse(uri: &str) -> Option<Report> {
    let (_, rest) = uri.split_once("://")?;
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    Some(Report { host: host.to_string(), path: percent_decode(path) })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A reader that passes everything through, stores each OSC 7 report in `slot` and raises
/// `alert` on notifications.
pub struct ScanReader<R> {
    inner: R,
    scanner: Scanner,
    slot: Slot,
    alert: Alert,
    agent: AgentSlot,
}

impl<R: Read> Read for ScanReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if let Some(report) = self.scanner.feed(&buf[..n])
            && let Ok(mut slot) = self.slot.lock()
        {
            *slot = Some(report);
        }
        if std::mem::take(&mut self.scanner.notified) {
            self.alert.raise();
        }
        if let Some(id) = self.scanner.agent.take()
            && let Ok(mut agent) = self.agent.lock()
        {
            *agent = (!id.is_empty()).then_some(id);
        }
        Ok(n)
    }
}

#[cfg(unix)]
pub use pty::ScanPty;

#[cfg(unix)]
mod pty {
    use std::fs::File;
    use std::io;
    use std::sync::Arc;

    use alacritty_terminal::event::{OnResize, WindowSize};
    use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
    use polling::{Event, PollMode, Poller};

    use super::{AgentSlot, Alert, ScanReader, Scanner, Slot};

    /// A [`Pty`] whose output is scanned for OSC 7 and notifications. Reads go through a second handle on the
    /// same PTY; registration, writes, resizes and child events go to the [`Pty`] itself.
    pub struct ScanPty {
        pty: Pty,
        reader: ScanReader<File>,
    }

    impl ScanPty {
        pub fn new(mut pty: Pty, slot: Slot, alert: Alert, agent: AgentSlot) -> io::Result<Self> {
            let file = pty.reader().try_clone()?;
            Ok(Self { pty, reader: ScanReader { inner: file, scanner: Scanner::default(), slot, alert, agent } })
        }
    }

    impl EventedReadWrite for ScanPty {
        type Reader = ScanReader<File>;
        type Writer = File;

        unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
            // SAFETY: the caller's guarantee carries over; `pty` lives as long as `self`.
            unsafe { self.pty.register(poll, interest, mode) }
        }

        fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
            self.pty.reregister(poll, interest, mode)
        }

        fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
            self.pty.deregister(poll)
        }

        fn reader(&mut self) -> &mut Self::Reader {
            &mut self.reader
        }

        fn writer(&mut self) -> &mut File {
            self.pty.writer()
        }
    }

    impl EventedPty for ScanPty {
        fn next_child_event(&mut self) -> Option<ChildEvent> {
            self.pty.next_child_event()
        }
    }

    impl OnResize for ScanPty {
        fn on_resize(&mut self, size: WindowSize) {
            self.pty.on_resize(size)
        }
    }
}

/// The directory in a title like `user@host: ~/src` (what Debian and Ubuntu's bash sets for
/// xterm-style terminals), for remote shells that don't send OSC 7.
pub fn title_dir(title: &str) -> Option<&str> {
    let (who, dir) = title.split_once(": ")?;
    let (user, host) = who.split_once('@')?;
    let word = |s: &str| !s.is_empty() && !s.contains(char::is_whitespace);
    let dir = dir.trim_end();
    (word(user) && word(host) && (dir.starts_with('/') || dir == "~" || dir.starts_with("~/"))).then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(host: &str, path: &str) -> Option<Report> {
        Some(Report { host: host.into(), path: path.into() })
    }

    #[test]
    fn finds_bel_and_st_terminated_reports() {
        let mut s = Scanner::default();
        assert_eq!(s.feed(b"hi\x1b]7;file://box/home/me\x07$ "), report("box", "/home/me"));
        assert_eq!(s.feed(b"\x1b]7;file://box/tmp\x1b\\"), report("box", "/tmp"));
        assert_eq!(s.feed(b"\x1b]7;file:///srv\x07"), report("", "/srv"));
    }

    #[test]
    fn report_split_across_reads() {
        let mut s = Scanner::default();
        let whole = b"\x1b[0m\x1b]7;file://box/a%20b\x1b\\";
        for split in 0..whole.len() {
            let mut s2 = Scanner::default();
            let first = s2.feed(&whole[..split]);
            let second = s2.feed(&whole[split..]);
            assert_eq!(first.or(second), report("box", "/a b"), "split at {split}");
        }
        assert_eq!(s.feed(b"\x1b]7;fi"), None);
        assert_eq!(s.feed(b"le://h/x\x07"), report("h", "/x"));
    }

    #[test]
    fn ignores_other_oscs_and_overlong_ones() {
        let mut s = Scanner::default();
        assert_eq!(s.feed(b"\x1b]0;title\x07\x1b]8;;http://x/\x1b\\"), None);
        let mut long = b"\x1b]7;file://h/".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_OSC * 2));
        long.extend(b"\x07\x1b]7;file://h/ok\x07");
        assert_eq!(s.feed(&long), report("h", "/ok"));
    }

    #[test]
    fn last_report_in_a_chunk_wins() {
        let mut s = Scanner::default();
        assert_eq!(s.feed(b"\x1b]7;file://h/a\x07\x1b]7;file://h/b\x07"), report("h", "/b"));
    }

    fn notifies(chunks: &[&[u8]]) -> bool {
        let mut s = Scanner::default();
        for c in chunks {
            s.feed(c);
        }
        s.notified
    }

    #[test]
    fn finds_notifications() {
        assert!(notifies(&[b"\x1b]9;Claude needs your permission\x07"]));
        assert!(notifies(&[b"\x1b]9;1 file changed\x1b\\"]));
        assert!(notifies(&[b"\x1b]777;notify;Claude Code;Waiting for input\x07"]));
        assert!(notifies(&[b"\x1b]99;i=1:d=0;Hello\x1b\\"]));
        assert!(notifies(&[b"\x1b]99;;hi\x07"]));
        assert!(notifies(&[b"out\x1b]9;he", b"llo\x07more"]));
    }

    #[test]
    fn ignores_non_notifications() {
        assert!(!notifies(&[b"\x1b]9;4;1;50\x07"])); // ConEmu progress
        assert!(!notifies(&[b"\x1b]9;4\x07"]));
        assert!(!notifies(&[b"\x1b]99;i=x:p=?;\x1b\\"])); // kitty capability query
        assert!(!notifies(&[b"\x1b]777;preexec\x07"]));
        assert!(!notifies(&[b"\x1b]0;title\x07\x1b]7;file://h/x\x07\x07"]));
    }

    #[test]
    fn notifications_leave_osc7_alone() {
        let mut s = Scanner::default();
        assert_eq!(s.feed(b"\x1b]7;file://h/a\x07\x1b]9;hi\x07"), report("h", "/a"));
        assert!(s.notified);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("/a%20b/%e2%82%ac"), "/a b/€");
        assert_eq!(percent_decode("/50%"), "/50%");
        assert_eq!(percent_decode("/%zz"), "/%zz");
    }

    #[test]
    fn titles() {
        assert_eq!(title_dir("me@devbox: ~/src/nex"), Some("~/src/nex"));
        assert_eq!(title_dir("me@devbox: /var/log"), Some("/var/log"));
        assert_eq!(title_dir("me@devbox: ~"), Some("~"));
        assert_eq!(title_dir("vim: notes.md"), None);
        assert_eq!(title_dir("me@devbox: notes"), None);
        assert_eq!(title_dir("a b@devbox: /x"), None);
    }
}
