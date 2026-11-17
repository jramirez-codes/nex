//! This machine's microphone, shared with the host a pane's `ssh` is connected to, as that
//! host's default ALSA capture device. Programs there that record from the default mic
//! (Claude Code's voice mode, `arecord`, SoX) then hear this machine's mic. The host needs
//! nothing installed: nex adds a block to `~/.asoundrc`, and runs only `sh` and `cat`.
//!
//! ```text
//! mic ─(audio.rs)─▶ nex ─▶ stdin of a waiting ssh session ─▶ cat ─▶ ~/.nex/mic.fifo
//!                                    ─▶ ALSA "file" plugin (pcm.!default capture) ─▶ the program
//! ```
//!
//! nex keeps one *waiting* session open per shared host ([`WAIT_SCRIPT`]). It makes the FIFO
//! and opens it for writing, which blocks until a program opens the mic. Then it removes the
//! FIFO's name, says `ready`, and copies its stdin into it; only then does nex start recording.
//! When the program closes the mic, `cat` gets a broken pipe, the session ends, the local mic
//! stops, and nex starts the next waiting session. While no session is waiting there is no
//! FIFO, so a program opening the mic gets an error rather than hanging.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{SshTarget, last_line, needs_login};
use crate::audio::{self, Capture, CaptureError};

/// The first line of nex's block in `~/.asoundrc`, and the last.
const MARK: &str = "# Added by nex: microphone";
const END_MARK: &str = "# End of nex microphone";

/// What the remote host can't do, or why sharing stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum MicError {
    /// The host wants a password (or another prompt) nex's connection can't answer.
    NeedsLogin,
    /// Only Linux hosts have ALSA.
    NotLinux,
    /// The host has no ALSA library (`libasound.so.2`), so no program there can record.
    NoAlsa,
    /// This machine's microphone couldn't be opened.
    Capture(CaptureError),
    Other(String),
}

impl std::fmt::Display for MicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsLogin => f.write_str("the host wants a password"),
            Self::NotLinux => f.write_str("only Linux hosts can use a shared microphone"),
            Self::NoAlsa => f.write_str("the host has no ALSA library (libasound2)"),
            Self::Capture(e) => write!(f, "{e}"),
            Self::Other(e) => f.write_str(e),
        }
    }
}

/// Is the host Linux with the ALSA library? Prints `not-linux`, `no-alsa`, or nothing.
const CHECK_SCRIPT: &str = r#"[ "$(uname -s)" = Linux ] || { echo not-linux; exit 0; }
for d in /usr/lib /usr/lib64 /lib /lib64 /usr/lib/*-linux-gnu /lib/*-linux-gnu /usr/local/lib; do
  [ -e "$d/libasound.so.2" ] && exit 0
done
echo no-alsa"#;

/// Adds nex's block to `~/.asoundrc` (once) and makes `~/.nex`. Prints `ok`.
///
/// The block makes capture from the default device read raw audio, in the wire format, from
/// `~/.nex/mic.fifo`; `plug` converts it to whatever a program asks for. Playback goes to
/// `sysdefault`, the card's own default, as it would without the block.
fn install_script() -> String {
    let block = format!(
        r#"{MARK}: capture from the default device reads the microphone of
# the nex window you're ssh'd in from. Remove through the end line, or use "Stop sharing microphone".
pcm.!default {{
  type asym
  playback.pcm "sysdefault"
  capture.pcm "nex_mic"
}}
pcm.nex_mic {{
  type plug
  slave {{
    pcm "nex_mic_fifo"
    format S16_LE
    rate {rate}
    channels 1
  }}
}}
pcm.nex_mic_fifo {{
  type file
  slave.pcm "null"
  file "/dev/null"
  infile "$HOME/.nex/mic.fifo"
  format "raw"
}}
{END_MARK}"#,
        rate = audio::RATE
    );
    // The here-document's delimiter is unquoted, so `$HOME` is written out as the path.
    format!(
        r#"mkdir -p "$HOME/.nex" && chmod 700 "$HOME/.nex" || exit 1
rc="$HOME/.asoundrc"
if ! grep -qsF '{MARK}' "$rc"; then
  [ -s "$rc" ] && printf '\n' >> "$rc"
  cat >> "$rc" <<NEX_EOF || exit 1
{block}
NEX_EOF
fi
echo ok"#
    )
}

/// Takes nex's block out of `~/.asoundrc` (keeping the file itself, and removing it only if
/// nothing else was in it), and any FIFO left behind.
fn remove_script() -> String {
    format!(
        r#"rm -f "$HOME/.nex/mic.fifo"
rc="$HOME/.asoundrc"
[ -f "$rc" ] || exit 0
grep -qsF '{MARK}' "$rc" || exit 0
awk 'index($0, "{MARK}") == 1 {{ skip = 1 }} !skip {{ print }} index($0, "{END_MARK}") == 1 {{ skip = 0 }}' "$rc" > "$rc.nex" || exit 1
cat "$rc.nex" > "$rc" && rm -f "$rc.nex"
grep -q '[^[:space:]]' "$rc" || rm -f "$rc""#
    )
}

/// The waiting session: see the module comment. Signals end it with the FIFO removed.
const WAIT_SCRIPT: &str = r#"f="$HOME/.nex/mic.fifo"
rm -f "$f"
mkfifo -m 600 "$f" || exit 1
trap 'rm -f "$f"' EXIT
trap 'exit 1' HUP INT TERM PIPE
exec 3>"$f"
rm -f "$f"
echo ready
cat >&3"#;

/// Runs a short script on the host and returns its output.
fn run(target: &SshTarget, script: &str) -> Result<String, MicError> {
    let out = target
        .batch_command(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| MicError::Other(format!("couldn't run ssh: {e}")))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if needs_login(&err) {
            return Err(MicError::NeedsLogin);
        }
        return Err(MicError::Other(last_line(&err).unwrap_or_else(|| format!("ssh exited with {}", out.status))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Check the host and add nex's block to its `~/.asoundrc`. Blocks: run it on a worker thread.
pub fn install(target: &SshTarget) -> Result<(), MicError> {
    let out = run(target, &format!("{CHECK_SCRIPT}\n{}", install_script()))?;
    match out.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some("ok") => Ok(()),
        Some("not-linux") => Err(MicError::NotLinux),
        Some("no-alsa") => Err(MicError::NoAlsa),
        _ => Err(MicError::Other("the host didn't finish setting up the microphone".into())),
    }
}

/// Take nex's block back out of the host's `~/.asoundrc`. Blocks: run it on a worker thread.
pub fn remove(target: &SshTarget) -> Result<(), MicError> {
    run(target, &remove_script()).map(|_| ())
}

/// What a [`Share`] reports, in order.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// The host is set up and waiting for a program to open the mic.
    Waiting,
    /// A program there opened the mic: this machine's mic is recording and streaming.
    Live,
    /// Sharing stopped for good (it doesn't retry). Not sent when stopped with [`Share::stop`].
    Failed(MicError),
}

/// The microphone shared with one host, from a worker thread, until stopped or dropped.
pub struct Share {
    stop: Arc<AtomicBool>,
    /// The session running now, to kill when stopping.
    session: Arc<Mutex<Option<Child>>>,
    pub events: Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Share {
    /// Set up `target` and start waiting for programs there to open the mic. `wake` is called
    /// after each event, to repaint the window.
    pub fn start(target: SshTarget, wake: impl Fn() + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let session = Arc::new(Mutex::new(None));
        let (tx, events) = mpsc::channel();
        let worker = Worker { target, stop: stop.clone(), session: session.clone(), tx, wake: Box::new(wake) };
        match std::thread::Builder::new().name("nex-mic-share".into()).spawn(move || worker.run()) {
            Ok(thread) => Self { stop, session, events, thread: Some(thread) },
            Err(e) => {
                let (tx, rx) = mpsc::channel();
                let _ = tx.send(Event::Failed(MicError::Other(e.to_string())));
                Self { stop, session, events: rx, thread: None }
            }
        }
    }

    /// Stop sharing: end the session (and with it any recording). The worker then takes nex's
    /// block back out of the host's `~/.asoundrc` and finishes; dropping the `Share` does this too.
    pub fn halt(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(child) = self.session.lock().ok().as_deref_mut().and_then(Option::as_mut) {
            let _ = child.kill();
        }
    }

    /// The worker has finished, host cleaned up (after [`halt`](Self::halt)).
    pub fn done(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Drop for Share {
    fn drop(&mut self) {
        self.halt();
    }
}

struct Worker {
    target: SshTarget,
    stop: Arc<AtomicBool>,
    session: Arc<Mutex<Option<Child>>>,
    tx: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
}

/// Waiting sessions that end without a program opening the mic, in a row, before giving up.
const MAX_FAILURES: u32 = 3;

impl Worker {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        (self.wake)();
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn run(self) {
        if let Err(e) = install(&self.target) {
            if !self.stopped() {
                self.send(Event::Failed(e));
            }
            return;
        }
        let mut failures = 0;
        while !self.stopped() {
            self.send(Event::Waiting);
            match self.serve_once() {
                Ok(()) => failures = 0,
                Err(_) if self.stopped() => break,
                Err(e @ (MicError::NeedsLogin | MicError::Capture(_))) => {
                    self.send(Event::Failed(e));
                    break;
                }
                Err(e) => {
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        self.send(Event::Failed(e));
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
        let _ = remove(&self.target);
    }

    /// One waiting session: wait for a program to open the mic, then stream until it closes it.
    fn serve_once(&self) -> Result<(), MicError> {
        let mut child = self
            .target
            .batch_command(WAIT_SCRIPT)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| MicError::Other(format!("couldn't run ssh: {e}")))?;
        let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
            let _ = child.kill();
            return Err(MicError::Other("couldn't talk to ssh".into()));
        };
        // Kept here so `Share::drop` can end a session blocked on waiting.
        if let Ok(mut s) = self.session.lock() {
            *s = Some(child);
        }
        // `Share::halt` sets the flag before looking for the session, so if it ran before the
        // session was stored, the flag shows it here.
        if self.stopped() {
            if let Some(child) = self.session.lock().ok().as_deref_mut().and_then(Option::as_mut) {
                let _ = child.kill();
            }
        }
        let result = self.stream(stdout, stderr, &mut stdin);
        drop(stdin);
        if let Some(mut child) = self.session.lock().ok().and_then(|mut s| s.take()) {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }

    fn stream(&self, stdout: impl Read, mut stderr: impl Read, stdin: &mut impl Write) -> Result<(), MicError> {
        let mut line = String::new();
        let started = Instant::now();
        let read = BufReader::new(stdout).read_line(&mut line);
        if line.trim() != "ready" {
            let mut err = String::new();
            let _ = stderr.read_to_string(&mut err);
            if needs_login(&err) {
                return Err(MicError::NeedsLogin);
            }
            let why = last_line(&err).or_else(|| read.err().map(|e| e.to_string()));
            // A session that waited a long time and then ended (the connection dropped, say)
            // isn't a failure to count against retrying.
            if why.is_none() && started.elapsed() > Duration::from_secs(30) {
                return Ok(());
            }
            return Err(MicError::Other(why.unwrap_or_else(|| "the host stopped waiting for the microphone".into())));
        }
        if self.stopped() {
            return Ok(());
        }
        let (tx, audio) = mpsc::channel();
        let capture = Capture::start(tx).map_err(MicError::Capture)?;
        self.send(Event::Live);
        loop {
            if self.stopped() || capture.failed() {
                return Ok(());
            }
            match audio.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    // The program closed the mic: the session is ending.
                    if stdin.write_all(&chunk).and_then(|()| stdin.flush()).is_err() {
                        return Ok(());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn home(name: &str) -> PathBuf {
        let h = std::env::temp_dir().join(format!("nex-mic-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        std::fs::create_dir_all(&h).unwrap();
        h
    }

    fn sh(home: &Path, script: &str) -> String {
        let out = Command::new("sh").args(["-c", script]).env("HOME", home).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn install_and_remove_keep_the_users_config() {
        let h = home("rc");
        let rc = h.join(".asoundrc");
        let mine = "pcm.mine {\n  type hw\n  card 1\n}\n";
        std::fs::write(&rc, mine).unwrap();
        assert_eq!(sh(&h, &install_script()).trim(), "ok");
        assert_eq!(sh(&h, &install_script()).trim(), "ok");
        let installed = std::fs::read_to_string(&rc).unwrap();
        assert!(installed.starts_with(mine));
        assert_eq!(installed.matches(MARK).count(), 1, "{installed}");
        assert!(installed.contains(&format!("infile \"{}/.nex/mic.fifo\"", h.display())), "{installed}");
        assert!(installed.contains(&format!("rate {}", audio::RATE)));
        assert!(h.join(".nex").is_dir());
        sh(&h, &remove_script());
        assert_eq!(std::fs::read_to_string(&rc).unwrap().trim_end(), mine.trim_end());
        std::fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn remove_deletes_a_file_it_made() {
        let h = home("made");
        sh(&h, &install_script());
        sh(&h, &remove_script());
        assert!(!h.join(".asoundrc").exists());
        sh(&h, &remove_script());
        std::fs::remove_dir_all(&h).unwrap();
    }

    /// The waiting session, run here with a local `sh` standing in for ssh and a reader
    /// standing in for a program opening the mic.
    #[test]
    fn wait_script_streams_until_the_reader_closes() {
        let h = home("wait");
        std::fs::create_dir_all(h.join(".nex")).unwrap();
        let fifo = h.join(".nex").join("mic.fifo");
        let mut child = Command::new("sh")
            .args(["-c", WAIT_SCRIPT])
            .env("HOME", &h)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while !fifo.exists() {
            assert!(start.elapsed() < Duration::from_secs(5), "no fifo");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut reader = std::fs::File::open(&fifo).unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(line, "ready\n");
        // Its name is gone once a program has it open: a second one gets an error, not a hang.
        assert!(!fifo.exists());
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"audio").unwrap();
        stdin.flush().unwrap();
        let mut got = [0u8; 5];
        reader.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"audio");
        drop(reader);
        // The next write hits the closed pipe and the session ends.
        let _ = stdin.write_all(&[0u8; 65536]);
        let _ = stdin.flush();
        drop(stdin);
        assert!(!child.wait().unwrap().success());
        std::fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn scripts_go_through_ssh_as_one_word() {
        let t = SshTarget { opts: vec!["-p".into(), "2222".into()], destination: "me@host".into() };
        let args = t.batch_args(WAIT_SCRIPT);
        assert_eq!(&args[args.len() - 3..args.len() - 1], ["--", "me@host"]);
        let last = args.last().unwrap();
        assert!(last.starts_with("sh -c 'f=\"$HOME/.nex/mic.fifo\""), "{last}");
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
    }
}
