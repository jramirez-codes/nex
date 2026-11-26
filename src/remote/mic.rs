//! This machine's microphone, shared with the host a pane's `ssh` is connected to, as that
//! host's default microphone. Programs there that record from the default mic (Claude Code's
//! voice mode, `arecord`, SoX) then hear this machine's mic. The host needs nothing installed.
//!
//! The host's ALSA has no clock of its own to read a FIFO by, so where it can, nex makes the
//! mic a real source in the host's sound server instead:
//!
//! ```text
//! PipeWire:  mic ─(audio.rs)─▶ nex ─▶ ssh ─▶ pw-cat (an Audio/Source node, made the default)
//!                                          ─▶ PipeWire ─▶ the program (ALSA, Pulse or PipeWire)
//! ALSA only: mic ─(audio.rs)─▶ nex ─▶ ssh ─▶ dd ─▶ ~/.nex/mic.fifo
//!                                          ─▶ ALSA "file" plugin (pcm.!default capture) ─▶ the program
//! ```
//!
//! **PipeWire** ([`pipewire_script`]): one session runs `pw-cat`, which PipeWire reads at the
//! graph's pace (silence when nex sends nothing), and `pw-link -m`, whose output tells nex
//! when a program links to the source. nex records only while one is linked, and starts each
//! recording with [`LEAD`] of silence, a cushion against the network's jitter.
//!
//! **ALSA** ([`wait_script`]), for hosts without PipeWire: a *waiting* session makes the FIFO
//! and opens it for writing, which blocks until a program opens the mic. Then it removes the
//! FIFO's name, says `ready`, and copies its stdin into it; only then does nex start recording
//! (and start the next waiting session, so the program can open the mic again right away).
//! When the program closes the mic, `dd` gets a broken pipe and the session ends. ALSA's file
//! plugin doesn't wait for a full read: what the FIFO lacks it fills with silence. So `dd`
//! writes whole [`BLOCK`]s, `arecord`'s own read size, which ALSA gets in one read.
//!
//! Each script runs over one of two connections ([`Transport`]): the system `ssh` with the
//! pane's options, or, for the host the window connects to itself (Android, which has no
//! `ssh`), a channel of nex's own client (`remote::ssh`).

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::remote::audio::{self, Capture, CaptureError};
use crate::remote::error::{last_line, needs_login};
use crate::remote::ssh::host::RemoteHost;
use crate::remote::transport::{Process, Running, Transport};
use crate::remote::{RemoteError, SshTarget};

/// The first line of nex's block in `~/.asoundrc`, and the last.
const MARK: &str = "# Added by nex: microphone";
const END_MARK: &str = "# End of nex microphone";

/// The PipeWire node nex's microphone is on the host.
const NODE: &str = "nex_mic";

/// Bytes `dd` writes into the FIFO at a time: 125 ms, the period `arecord` reads by default
/// (a quarter of its 500 ms buffer). Under `PIPE_BUF`, so each one arrives whole.
const BLOCK: u32 = audio::RATE / 8 * 2;

/// Silence sent ahead of each recording on PipeWire, so audio arriving a little late (the
/// network) is already queued when PipeWire reads it, rather than replaced by silence. It
/// delays the whole recording by as much, so this much at its end is still on the way when
/// the program stops. 120 ms still had drops on a LAN; 200 ms had none.
const LEAD: Duration = Duration::from_millis(200);

/// What the remote host can't do, or why sharing stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum MicError {
    /// The connection failed, or the host wants a password nex's connection can't answer.
    Remote(RemoteError),
    /// Only Linux hosts have ALSA.
    NotLinux,
    /// The host has no ALSA library (`libasound.so.2`), so no program there can record.
    NoAlsa,
    /// This machine's microphone couldn't be opened.
    Capture(CaptureError),
}

impl From<RemoteError> for MicError {
    fn from(e: RemoteError) -> Self {
        MicError::Remote(e)
    }
}

impl std::fmt::Display for MicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Remote(RemoteError::NeedsLogin) => f.write_str("the host wants a password"),
            Self::Remote(RemoteError::Other(e)) => f.write_str(e),
            Self::NotLinux => f.write_str("only Linux hosts can use a shared microphone"),
            Self::NoAlsa => f.write_str("the host has no ALSA library (libasound2)"),
            Self::Capture(e) => write!(f, "{e}"),
        }
    }
}

/// How the host gets the microphone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    PipeWire,
    Alsa,
}

/// Picks the mode and gets the host ready for it. Prints `pipewire`, `ok` (ALSA, block added
/// to `~/.asoundrc`), `not-linux` or `no-alsa`.
///
/// With PipeWire, ALSA's default already goes to PipeWire, so an `~/.asoundrc` block (left by
/// an earlier nex) would take it away again: it comes out. Each step that can stop early runs
/// in a subshell, so its `exit` doesn't end this script.
fn setup_script() -> String {
    format!(
        r#"[ "$(uname -s)" = Linux ] || {{ echo not-linux; exit 0; }}
if command -v pw-cat >/dev/null 2>&1 && command -v pw-link >/dev/null 2>&1 \
  && command -v pw-metadata >/dev/null 2>&1 && pw-link -o >/dev/null 2>&1; then
  ( {remove} ) || exit 1
  echo pipewire
  exit 0
fi
alsa=
for d in /usr/lib /usr/lib64 /lib /lib64 /usr/lib/*-linux-gnu /lib/*-linux-gnu /usr/local/lib; do
  [ -e "$d/libasound.so.2" ] && {{ alsa=1; break; }}
done
[ -n "$alsa" ] || {{ echo no-alsa; exit 0; }}
( {install} )"#,
        remove = remove_script(),
        install = install_script(),
    )
}

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

/// Ends any waiting sessions whose ssh is gone (still blocked opening the FIFO), and takes
/// nex's block out of `~/.asoundrc` (keeping the file itself, and removing it only if nothing
/// else was in it), and any FIFO left behind.
fn remove_script() -> String {
    format!(
        r#"for p in "$HOME"/.nex/mic.wait.*; do
  [ -f "$p" ] || continue
  pid=$(cat "$p")
  [ -n "$pid" ] && grep -qs mic.fifo "/proc/$pid/cmdline" && kill -9 "$pid" 2>/dev/null
  rm -f "$p"
done
rm -f "$HOME/.nex/mic.fifo"
rc="$HOME/.asoundrc"
[ -f "$rc" ] || exit 0
grep -qsF '{MARK}' "$rc" || exit 0
awk 'index($0, "{MARK}") == 1 {{ skip = 1 }} !skip {{ print }} index($0, "{END_MARK}") == 1 {{ skip = 0 }}' "$rc" > "$rc.nex" || exit 1
cat "$rc.nex" > "$rc" && rm -f "$rc.nex"
grep -q '[^[:space:]]' "$rc" || rm -f "$rc""#
    )
}

/// The PipeWire session: see the module comment. Makes nex's node the configured default
/// source while it runs, and puts back the one before it when it ends.
///
/// It ends when its input does (nex stopped, or the connection dropped). `pw-cat` alone
/// wouldn't: at the end of its input it waits for what it has queued to be played, which
/// never happens while nothing records. So `cat` feeds it through a FIFO, and its end ends
/// the script, `pw-cat` with it.
///
/// `node.always-process` keeps PipeWire reading the node while nothing records, so what was
/// still on the way when a program stopped is played to no one, not to the next program.
/// `--latency 20ms` makes `pw-cat` read 20 ms at a time: at its default, 100 ms, each read
/// could take more than the cushion holds.
fn pipewire_script() -> String {
    format!(
        r#"key=default.configured.audio.source
prev=$(pw-metadata 0 "$key" 2>/dev/null | sed -n "s/.*key:'$key' value:'\(.*\)' type:.*/\1/p")
case $prev in *'"{NODE}"'*) prev= ;; esac
d="$HOME/.nex"
mkdir -p "$d" && chmod 700 "$d" || exit 1
a="$d/mic.audio.$$"
rm -f "$a"
mkfifo -m 600 "$a" || exit 1
mon= pc=
finish() {{
  kill $mon $pc 2>/dev/null
  rm -f "$a"
  if [ -n "$prev" ]; then pw-metadata 0 "$key" "$prev" Spa:String:JSON; else pw-metadata -d 0 "$key"; fi >/dev/null 2>&1
}}
trap finish EXIT
trap 'exit 1' HUP INT TERM PIPE
pw-link -m -l -o 2>/dev/null &
mon=$!
pw-cat --playback --raw --format s16 --rate {rate} --channels 1 --latency 20ms \
  -P '{{ media.class = "Audio/Source" node.name = "{NODE}" node.description = "Microphone (shared by nex)" node.always-process = true }}' - <"$a" >/dev/null &
pc=$!
pw-metadata 0 "$key" '{{"name":"{NODE}"}}' Spa:String:JSON >/dev/null 2>&1
echo ready
cat >"$a""#,
        rate = audio::RATE
    )
}

/// The ALSA waiting session: see the module comment. Signals end it with the FIFO removed.
///
/// Once the FIFO is open its name is gone, and may soon be the next session's: from then on
/// the exit trap leaves it alone. The pid file lets [`remove_script`] end a session whose ssh
/// was killed while it waited (nothing else would: it's blocked opening the FIFO).
fn wait_script() -> String {
    format!(
        r#"f="$HOME/.nex/mic.fifo"
p="$HOME/.nex/mic.wait.$$"
echo $$ > "$p" || exit 1
trap 'rm -f "$f" "$p"' EXIT
trap 'exit 1' HUP INT TERM PIPE
rm -f "$f"
mkfifo -m 600 "$f" || exit 1
exec 3>"$f"
rm -f "$f"
trap 'rm -f "$p"' EXIT
echo ready
dd obs={BLOCK} 2>/dev/null >&3"#
    )
}

/// Runs a short script on the host and returns its output.
fn run(transport: &Transport, script: &str) -> Result<String, MicError> {
    Ok(String::from_utf8_lossy(&transport.output(script, None)?).into_owned())
}

/// Check the host and get it ready. Blocks: run it on a worker thread.
fn setup(transport: &Transport) -> Result<Mode, MicError> {
    parse_setup(&run(transport, &setup_script())?)
}

fn parse_setup(out: &str) -> Result<Mode, MicError> {
    match out.lines().map(str::trim).find(|l| !l.is_empty()) {
        Some("pipewire") => Ok(Mode::PipeWire),
        Some("ok") => Ok(Mode::Alsa),
        Some("not-linux") => Err(MicError::NotLinux),
        Some("no-alsa") => Err(MicError::NoAlsa),
        _ => Err(RemoteError::from("the host didn't finish setting up the microphone").into()),
    }
}

/// Take nex's block back out of the host's `~/.asoundrc`. Blocks: run it on a worker thread.
pub fn remove(transport: &Transport) -> Result<(), MicError> {
    run(transport, &remove_script()).map(|_| ())
}

/// Follows `pw-link -m -l -o`: is any program linked to (recording from) nex's node?
///
/// Each line starts with a mark (`+` added, `-` removed, `*` changed, ` ` as it was), then
/// either an output port, `node:port`, or a link from the port above it, `  |-> node:port`.
#[derive(Default)]
struct Links {
    /// The port above is one of nex's node's.
    ours: bool,
    peers: HashSet<String>,
}

impl Links {
    /// Read one line of `pw-link`'s output; returns whether anything is recording now.
    fn line(&mut self, line: &str) -> bool {
        let (mark, rest) = match line.chars().next() {
            Some(c @ ('+' | '-' | '*' | '=' | ' ')) => (c, &line[1..]),
            _ => (' ', line),
        };
        let body = rest.trim_start();
        if let Some(peer) = body.strip_prefix("|->") {
            if self.ours {
                let peer = peer.trim().to_string();
                if mark == '-' {
                    self.peers.remove(&peer);
                } else {
                    self.peers.insert(peer);
                }
            }
        } else if !body.starts_with("|<-") && !body.is_empty() {
            self.ours = body.strip_prefix(NODE).is_some_and(|p| p.starts_with(':'));
            if self.ours && mark == '-' {
                self.peers.clear();
            }
        }
        !self.peers.is_empty()
    }
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

/// The sessions running now (each with its [`Session::key`]), to end when stopping.
type Sessions = Arc<Mutex<Vec<(u64, Running)>>>;

/// Tells sessions apart in [`Sessions`].
static NEXT_KEY: AtomicU64 = AtomicU64::new(0);

/// The microphone shared with one host, from a worker thread, until stopped or dropped.
pub struct Share {
    stop: Arc<AtomicBool>,
    sessions: Sessions,
    pub events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Share {
    /// Set up `target` and start waiting for programs there to open the mic. `wake` is called
    /// after each event, to repaint the window.
    pub fn start(target: SshTarget, wake: impl Fn() + Send + 'static) -> Self {
        Self::start_via(Transport::Ssh(target), wake)
    }

    /// Like [`start`](Self::start), for the host the window is connected to itself, over
    /// nex's own connection to it.
    pub fn start_direct(host: Arc<RemoteHost>, wake: impl Fn() + Send + 'static) -> Self {
        Self::start_via(Transport::Nex(host), wake)
    }

    fn start_via(transport: Transport, wake: impl Fn() + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let sessions = Sessions::default();
        let (tx, events) = mpsc::channel();
        let worker = Worker { transport, stop: stop.clone(), sessions: sessions.clone(), tx, wake: Box::new(wake) };
        match std::thread::Builder::new().name("nex-mic-share".into()).spawn(move || worker.run()) {
            Ok(thread) => Self { stop, sessions, events, thread: Some(thread) },
            Err(e) => {
                let (tx, rx) = mpsc::channel();
                let _ = tx.send(Event::Failed(RemoteError::Other(e.to_string()).into()));
                Self { stop, sessions, events: rx, thread: None }
            }
        }
    }

    /// Stop sharing: end the sessions (and with them any recording). The worker then takes
    /// nex's block back out of the host's `~/.asoundrc` and finishes; dropping the `Share`
    /// does this too.
    pub fn halt(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, running) in sessions.iter_mut() {
                running.end();
            }
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

/// One session running a script on the host. Dropping it ends it.
struct Session {
    /// Its entry in [`Sessions`].
    key: u64,
    sessions: Sessions,
    stdin: Option<Box<dyn Write + Send>>,
    stdout: Option<Box<dyn Read + Send>>,
    /// Everything it writes to stderr, once it has ended (`ssh` only: a command on nex's own
    /// connection has none).
    stderr: Option<Receiver<String>>,
}

impl Session {
    fn kill(&mut self) {
        self.stdin.take();
        let running = self.sessions.lock().ok().and_then(|mut s| {
            let at = s.iter().position(|(k, _)| *k == self.key)?;
            Some(s.swap_remove(at).1)
        });
        if let Some(running) = running {
            running.stop();
        }
    }

    /// End the session and return what it wrote to stderr. That ends when the host ends the
    /// session, which a host that has stopped answering never does: it isn't waited for long.
    fn finish(mut self) -> String {
        self.kill();
        self.stderr.take().and_then(|r| r.recv_timeout(Duration::from_secs(2)).ok()).unwrap_or_default()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Why a session ended before it was asked to. One that ran a long time and then ended with
/// nothing to say (the connection dropped, say) isn't a failure to count against retrying.
fn ended(session: Session, started: Instant, fallback: &str) -> Result<(), MicError> {
    let err = session.finish();
    if needs_login(&err) {
        return Err(RemoteError::NeedsLogin.into());
    }
    match last_line(&err) {
        None if started.elapsed() > Duration::from_secs(30) => Ok(()),
        why => Err(RemoteError::Other(why.unwrap_or_else(|| fallback.into())).into()),
    }
}

/// This machine's mic, recording, and the audio it sends.
struct Recording {
    capture: Capture,
    audio: Receiver<Vec<u8>>,
}

impl Recording {
    fn start() -> Result<Self, MicError> {
        let (tx, audio) = mpsc::channel();
        Ok(Self { capture: Capture::start(tx).map_err(MicError::Capture)?, audio })
    }

    /// The next chunk of audio, waiting up to `wait`. A mic that stopped (unplugged, or
    /// switched) is opened again.
    fn next(&mut self, wait: Duration) -> Result<Option<Vec<u8>>, MicError> {
        if self.capture.failed() {
            std::thread::sleep(Duration::from_millis(200));
            *self = Self::start()?;
        }
        match self.audio.recv_timeout(wait) {
            Ok(chunk) => Ok(Some(chunk)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                *self = Self::start()?;
                Ok(None)
            }
        }
    }
}

/// `d` of silence in the wire format.
fn silence(d: Duration) -> Vec<u8> {
    vec![0; (u128::from(audio::RATE) * d.as_millis() / 1000) as usize * 2]
}

struct Worker {
    transport: Transport,
    stop: Arc<AtomicBool>,
    sessions: Sessions,
    tx: Sender<Event>,
    wake: Box<dyn Fn() + Send>,
}

/// Sessions that end without a program opening the mic, in a row, before giving up.
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
        let mode = match setup(&self.transport) {
            Ok(mode) => mode,
            Err(e) => {
                if !self.stopped() {
                    self.send(Event::Failed(e));
                }
                return;
            }
        };
        let mut failures = 0;
        // ALSA: the session waiting for the next program, started while one records.
        let mut next = None;
        while !self.stopped() {
            let result = match mode {
                Mode::PipeWire => self.serve_pipewire(),
                Mode::Alsa => self.serve_alsa(&mut next),
            };
            match result {
                Ok(()) => failures = 0,
                Err(_) if self.stopped() => break,
                Err(e @ (MicError::Remote(RemoteError::NeedsLogin) | MicError::Capture(_))) => {
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
        drop(next);
        let _ = remove(&self.transport);
    }

    /// Start `script` on the host, with its stdin and stdout for us and its stderr collected.
    fn spawn(&self, script: &str) -> Result<Session, MicError> {
        let key = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
        let Process { stdin, stdout, stderr, running } = self.transport.spawn(script, true)?;
        let session = Session { key, sessions: self.sessions.clone(), stdin, stdout: Some(stdout), stderr };
        if let Ok(mut s) = self.sessions.lock() {
            s.push((key, running));
        }
        // `Share::halt` sets the flag before ending the sessions, so if it ran before this
        // one was stored, the flag shows it here.
        if self.stopped() {
            drop(session);
            return Err(RemoteError::from("stopped").into());
        }
        if session.stdin.is_none() || session.stdout.is_none() {
            return Err(RemoteError::from("couldn't talk to ssh").into());
        }
        Ok(session)
    }

    /// One PipeWire session: the source stays up, and the mic records while something
    /// records from it, until the session ends.
    fn serve_pipewire(&self) -> Result<(), MicError> {
        let mut session = self.spawn(&pipewire_script())?;
        let started = Instant::now();
        let stdout = session.stdout.take().expect("spawn checks stdout");
        let (line_tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });
        let fallback = "the host stopped sharing the microphone";
        let mut links = Links::default();
        let mut live = false;
        // Until `ready`, `pw-link`'s first listing may come first.
        loop {
            match lines.recv_timeout(Duration::from_millis(200)) {
                Ok(l) if l.trim() == "ready" => break,
                Ok(l) => live = links.line(&l),
                Err(RecvTimeoutError::Timeout) if self.stopped() => return Ok(()),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return ended(session, started, fallback),
            }
        }
        self.send(Event::Waiting);
        let mut stdin = session.stdin.take().expect("spawn checks stdin");
        let mut recording: Option<Recording> = None;
        // [`LEAD`] is still to be sent ahead of the recording's first audio.
        let mut lead = false;
        loop {
            if self.stopped() {
                return Ok(());
            }
            // While recording, only look; otherwise wait here for something to link. Then take
            // every line that's waiting before acting on where they leave things.
            let mut line = if recording.is_some() {
                lines.try_recv()
            } else {
                lines.recv_timeout(Duration::from_millis(200)).map_err(|e| match e {
                    RecvTimeoutError::Timeout => TryRecvError::Empty,
                    RecvTimeoutError::Disconnected => TryRecvError::Disconnected,
                })
            };
            loop {
                match line {
                    Ok(l) => live = links.line(&l),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        drop(stdin);
                        return ended(session, started, fallback);
                    }
                }
                line = lines.try_recv();
            }
            match (&mut recording, live) {
                (None, true) => {
                    recording = Some(Recording::start()?);
                    lead = true;
                    self.send(Event::Live);
                }
                (Some(_), false) => {
                    recording = None;
                    self.send(Event::Waiting);
                }
                (Some(r), true) => {
                    // Everything that's ready, then back to look at the links.
                    let mut wait = Duration::from_millis(20);
                    while let Some(mut chunk) = r.next(wait)? {
                        // The cushion goes with the first audio, not when the mic opens: the
                        // time it takes to open would use it up.
                        if std::mem::take(&mut lead) {
                            chunk.splice(0..0, silence(LEAD));
                        }
                        if stdin.write_all(&chunk).and_then(|()| stdin.flush()).is_err() {
                            drop(stdin);
                            return ended(session, started, fallback);
                        }
                        wait = Duration::ZERO;
                    }
                }
                (None, false) => {}
            }
        }
    }

    /// One ALSA waiting session: wait for a program to open the mic, then stream until it
    /// closes it. `next` is the session waiting for the program after it, if one was started.
    fn serve_alsa(&self, next: &mut Option<Session>) -> Result<(), MicError> {
        let mut session = match next.take() {
            Some(s) => s,
            None => self.spawn(&wait_script())?,
        };
        self.send(Event::Waiting);
        let started = Instant::now();
        let mut line = String::new();
        let read = BufReader::new(session.stdout.take().expect("spawn checks stdout")).read_line(&mut line);
        if line.trim() != "ready" {
            let fallback = read.err().map_or_else(|| "the host stopped waiting for the microphone".into(), |e| e.to_string());
            return ended(session, started, &fallback);
        }
        if self.stopped() {
            return Ok(());
        }
        // The program may close the mic and open it again straight away (a probe first, say):
        // the next FIFO is ready for it.
        *next = self.spawn(&wait_script()).ok();
        let mut stdin = session.stdin.take().expect("spawn checks stdin");
        let mut recording = Recording::start()?;
        self.send(Event::Live);
        let mut last = Instant::now();
        loop {
            if self.stopped() {
                return Ok(());
            }
            let chunk = match recording.next(Duration::from_millis(100))? {
                Some(chunk) => chunk,
                // The mic has gone quiet (it's being opened again): keep the program's reads
                // moving with silence, and find out when it closes the mic.
                None if last.elapsed() > Duration::from_millis(300) => silence(Duration::from_millis(100)),
                None => continue,
            };
            last = Instant::now();
            // The program closed the mic: the session is ending.
            if stdin.write_all(&chunk).and_then(|()| stdin.flush()).is_err() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    #[cfg(target_os = "linux")]
    use std::process::{Child, Stdio};

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

    /// The setup's early exits (an older version's ALSA check exited 0 when it passed) must
    /// not stop it before it says what the host has.
    #[test]
    fn setup_reports_a_mode() {
        let h = home("setup");
        let out = sh(&h, &setup_script());
        match parse_setup(&out) {
            Ok(Mode::Alsa) => assert!(h.join(".asoundrc").exists()),
            // PipeWire takes out a block left by an ALSA share.
            Ok(Mode::PipeWire) => {
                std::fs::create_dir_all(h.join(".nex")).unwrap();
                sh(&h, &install_script());
                assert_eq!(parse_setup(&sh(&h, &setup_script())), Ok(Mode::PipeWire));
                assert!(!h.join(".asoundrc").exists());
            }
            Err(MicError::NotLinux | MicError::NoAlsa) => assert!(!cfg!(target_os = "linux") || out.contains("no-alsa")),
            Err(e) => panic!("{e}: {out}"),
        }
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

    #[cfg(target_os = "linux")]
    fn start_waiting(h: &Path) -> Child {
        Command::new("sh")
            .args(["-c", &wait_script()])
            .env("HOME", h)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[cfg(target_os = "linux")]
    fn wait_for(path: &Path) {
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < Duration::from_secs(5), "no {}", path.display());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The waiting session, run here with a local `sh` standing in for ssh and a reader
    /// standing in for a program opening the mic. Linux only, as the hosts it runs on are:
    /// `remove_script` finds a waiting session through `/proc`, so elsewhere the next one is
    /// never ended and this waits forever (it hung the v0.11.0 macOS release).
    #[test]
    #[cfg(target_os = "linux")]
    fn wait_script_streams_whole_blocks_until_the_reader_closes() {
        let h = home("wait");
        std::fs::create_dir_all(h.join(".nex")).unwrap();
        let fifo = h.join(".nex").join("mic.fifo");
        let mut child = start_waiting(&h);
        wait_for(&fifo);
        let mut reader = std::fs::File::open(&fifo).unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(line, "ready\n");
        // Its name is gone once a program has it open, ready for the next session.
        assert!(!fifo.exists());
        let mut next = start_waiting(&h);
        wait_for(&fifo);
        let mut stdin = child.stdin.take().unwrap();
        // Written in odd pieces, read in whole blocks: ALSA would pad a short read.
        let audio: Vec<u8> = (0..BLOCK).map(|i| i as u8).collect();
        for piece in audio.chunks(333) {
            stdin.write_all(piece).unwrap();
            stdin.flush().unwrap();
        }
        let mut got = vec![0u8; BLOCK as usize];
        assert_eq!(reader.read(&mut got).unwrap(), BLOCK as usize);
        assert_eq!(got, audio);
        drop(reader);
        // The next write hits the closed pipe and the session ends, leaving the next
        // session's FIFO in place.
        let _ = stdin.write_all(&vec![0u8; 65536]);
        let _ = stdin.flush();
        drop(stdin);
        assert!(!child.wait().unwrap().success());
        assert!(fifo.exists());
        // Its ssh gone, the next session would wait forever: removing ends it.
        sh(&h, &remove_script());
        assert!(!next.wait().unwrap().success());
        assert!(std::fs::read_dir(h.join(".nex")).unwrap().next().is_none());
        std::fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn links_follow_programs_recording_from_the_node() {
        // `pw-link -m -l -o` as a recorder comes and goes.
        let lines = [
            ("+alsa_output.pci:monitor_FL", false),
            ("+  |-> pw-record:input_FL", false),
            ("+nex_mic:capture_MONO", false),
            ("+pw-record:monitor_MONO", false),
            ("*nex_mic:capture_MONO", false),
            ("+  |-> pw-record:input_MONO", true),
            ("*nex_mic:capture_MONO", true),
            ("   |-> pw-record:input_MONO", true),
            ("*nex_mic:capture_MONO", true),
            ("+  |-> arecord:input_MONO", true),
            ("-  |-> pw-record:input_MONO", true),
            ("-pw-record:monitor_MONO", true),
            ("*nex_mic:capture_MONO", true),
            ("-  |-> arecord:input_MONO", false),
            ("*nex_mic_other:capture_MONO", false),
            ("+  |-> pw-record:input_MONO", false),
        ];
        let mut links = Links::default();
        for (line, live) in lines {
            assert_eq!(links.line(line), live, "{line}");
        }
    }

    #[test]
    fn scripts_go_through_ssh_as_one_word() {
        let t = SshTarget { opts: vec!["-p".into(), "2222".into()], destination: "me@host".into() };
        let args = t.batch_args(&wait_script());
        assert_eq!(&args[args.len() - 3..args.len() - 1], ["--", "me@host"]);
        let last = args.last().unwrap();
        assert!(last.starts_with("sh -c 'f=\"$HOME/.nex/mic.fifo\""), "{last}");
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
    }

    /// Over nex's own connection a script goes through the login shell as one `sh -c` word.
    #[test]
    fn direct_scripts_reach_sh_whole() {
        let h = home("direct");
        let script = wait_script();
        // The login shell (here `sh`) hands the script, quotes and all, to the `sh` it runs.
        let echo = crate::remote::ssh::sh_command(&format!("cat <<'NEX_EOF'\n{script}\nNEX_EOF"));
        assert_eq!(sh(&h, &echo).trim_end(), script);
        std::fs::remove_dir_all(&h).unwrap();
    }

    #[test]
    fn lead_is_whole_samples() {
        assert_eq!(silence(LEAD).len(), (audio::RATE as usize * LEAD.as_millis() as usize / 1000) * 2);
        assert_eq!(BLOCK, 4000);
    }
}
