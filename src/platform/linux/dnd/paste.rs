//! Pastes read off the UI thread.
//!
//! The app holding the clipboard sends a paste down a pipe, in its own time (the wait is capped
//! at `PASTE_TIMEOUT`, and `IMAGE_TIMEOUT` for an image). So a paste key only asks for the data
//! ([`Pipes`], made by `Dnd::feed`); a worker thread reads it ([`Pipes::read`]) and wakes the
//! UI, and a later frame's `feed` puts the paste into egui's input ([`Queue::release`]).
//!
//! **Order.** The app must see input in the order it was typed, so while a paste is being read
//! the keyboard events that follow it (keys, text, IME, copy / cut / paste) are held back, and
//! go to the app after it, on later frames:
//!
//! - A paste asked for first lands first, whichever read finishes first.
//! - A key typed after a paste key is never handled before that paste: Enter after Ctrl+V
//!   runs the pasted command, not an empty line with the paste after it.
//! - Held input is handled under the modifiers of the frame it was typed in (the app reads
//!   Shift and Alt from the frame, not the event), so a frame that takes input out of the
//!   queue reports those modifiers, and takes only input that shares them. A paste ends its
//!   frame, as the app handles one paste per frame.
//! - Only the keyboard is held. The pointer stays live, so a paste lands in the pane that has
//!   the focus when it arrives: clicking another pane during a slow read moves the paste there.
//!
//! A read that fails or times out lands as a paste with no text, which the app treats like a
//! clipboard that holds none (nothing is pasted and nothing is reported).

use std::collections::VecDeque;
use std::io::PipeReader;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use egui::{Event, Modifiers, RawInput};

use crate::file_explorer::fs::ops::parse_paths;
use crate::platform::ClipboardFiles;
use crate::platform::linux::dnd::{IMAGE_TIMEOUT, PASTE_TIMEOUT, log, read_until_closed};

/// The clipboard's contents on their way from the app holding it, one pipe per type asked for.
#[derive(Default)]
pub(super) struct Pipes {
    pub(super) text: Option<PipeReader>,
    /// Copied files, as a `text/uri-list`.
    pub(super) uris: Option<PipeReader>,
    pub(super) png: Option<PipeReader>,
}

/// What a paste read.
#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Read {
    /// The clipboard's text, if it holds any.
    pub(super) text: Option<String>,
    /// Copied files, or else an image, if they were asked for and it holds either.
    pub(super) files: Option<ClipboardFiles>,
}

impl Pipes {
    /// Read every pipe to its end. Blocks for as long as the clipboard's owner takes, up to
    /// the timeouts: for a worker thread.
    pub(super) fn read(self) -> Read {
        self.read_within(PASTE_TIMEOUT, IMAGE_TIMEOUT)
    }

    fn read_within(self, timeout: Duration, image_timeout: Duration) -> Read {
        let text = self
            .text
            .and_then(|pipe| read_until_closed(pipe, timeout))
            .map(|bytes| String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
            .filter(|text| !text.is_empty());
        let paths = self
            .uris
            .and_then(|pipe| read_until_closed(pipe, timeout))
            .map(|bytes| parse_paths(&String::from_utf8_lossy(&bytes)))
            .filter(|paths| !paths.is_empty());
        let files = match paths {
            Some(paths) => Some(ClipboardFiles::Paths(paths)),
            None => self.png.and_then(|pipe| read_until_closed(pipe, image_timeout)).filter(|png| !png.is_empty()).map(ClipboardFiles::Png),
        };
        log(format_args!(
            "paste: read {} bytes of text, {}",
            text.as_ref().map_or(0, String::len),
            match &files {
                Some(ClipboardFiles::Paths(paths)) => format!("{} file(s)", paths.len()),
                Some(ClipboardFiles::Png(png)) => format!("{} bytes of image", png.len()),
                None => "no files".into(),
            }
        ));
        Read { text, files }
    }
}

/// A paste that reached egui's input this frame.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Landed {
    /// Shift was held with the paste key.
    pub(super) shift: bool,
    /// It carried text: an `Event::Paste` was added to the frame's events.
    pub(super) text: bool,
    pub(super) files: Option<ClipboardFiles>,
}

enum Held {
    /// A paste being read, with the modifiers of the frame its key was pressed in.
    Paste { shift: bool, modifiers: Modifiers, read: Receiver<Read> },
    /// One frame's keyboard events, typed after a paste that hasn't landed.
    Keys(Vec<Event>, Modifiers),
}

/// Pastes being read, and the keyboard input typed since, in the order the app will see them
/// (see the module docs).
#[derive(Default)]
pub(super) struct Queue(VecDeque<Held>);

impl Queue {
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Before a frame: if a paste is still on its way, take the frame's keyboard events out
    /// of `raw`, to follow it.
    pub(super) fn hold(&mut self, raw: &mut RawInput) {
        if self.0.is_empty() {
            return;
        }
        let keyboard = |e: &Event| matches!(e, Event::Key { .. } | Event::Text(_) | Event::Ime(_) | Event::Paste(_) | Event::Copy | Event::Cut);
        let (held, rest): (Vec<Event>, Vec<Event>) = std::mem::take(&mut raw.events).into_iter().partition(keyboard);
        raw.events = rest;
        if !held.is_empty() {
            self.0.push_back(Held::Keys(held, raw.modifiers));
        }
    }

    /// A paste key was pressed, after everything held so far: read `pipes` on a worker thread,
    /// which calls `wake` when it's done (to draw a frame that lands the paste).
    pub(super) fn request(&mut self, pipes: Pipes, shift: bool, modifiers: Modifiers, wake: impl FnOnce() + Send + 'static) {
        let (done, read) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new().name("nex-paste".into()).spawn(move || {
            let _ = done.send(pipes.read());
            wake();
        });
        if let Err(e) = worker {
            // `done` went with the closure: the paste lands at once, with nothing read.
            log(format_args!("paste: no thread to read it on: {e}"));
        }
        self.0.push_back(Held::Paste { shift, modifiers, read });
    }

    /// Before a frame, after `hold`: move what is ready from the front of the queue into
    /// `raw`, up to and including the first paste. Stops at a paste still being read. Never
    /// waits.
    pub(super) fn release(&mut self, raw: &mut RawInput) -> Option<Landed> {
        // The modifiers of what this frame has taken out of the queue so far.
        let mut taken: Option<Modifiers> = None;
        loop {
            let modifiers = match self.0.front()? {
                Held::Paste { modifiers, .. } | Held::Keys(_, modifiers) => *modifiers,
            };
            if taken.is_some_and(|m| m != modifiers) {
                return None;
            }
            // A worker that died without an answer read nothing.
            let read = match self.0.front()? {
                Held::Paste { read, .. } => match read.try_recv() {
                    Ok(read) => Some(read),
                    Err(TryRecvError::Disconnected) => Some(Read::default()),
                    Err(TryRecvError::Empty) => return None,
                },
                Held::Keys(..) => None,
            };
            raw.modifiers = modifiers;
            taken = Some(modifiers);
            match (self.0.pop_front()?, read) {
                (Held::Paste { shift, .. }, Some(Read { text, files })) => {
                    let landed = Landed { shift, text: text.is_some(), files };
                    raw.events.extend(text.map(Event::Paste));
                    return Some(landed);
                }
                (Held::Keys(events, _), _) => raw.events.extend(events),
                (Held::Paste { .. }, None) => unreachable!("a paste is only taken once it was read"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::mpsc::channel;
    use std::time::Instant;

    use egui::Key;

    use super::*;

    const SOON: Duration = Duration::from_secs(5);

    fn key(key: Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    fn frame(events: Vec<Event>) -> RawInput {
        RawInput { events, ..Default::default() }
    }

    /// A pipe holding `bytes`, already closed by its writer.
    fn sent(bytes: &[u8]) -> Option<PipeReader> {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer.write_all(bytes).unwrap();
        Some(reader)
    }

    /// Ask for a paste of `pipes`; the receiver hears when its read is done.
    fn request(queue: &mut Queue, pipes: Pipes, shift: bool) -> Receiver<()> {
        let (woke, wake) = channel();
        queue.request(pipes, shift, Modifiers { shift, ..Modifiers::NONE }, move || woke.send(()).unwrap());
        wake
    }

    #[test]
    fn a_slow_clipboard_owner_does_not_hold_up_frames() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let mut queue = Queue::default();
        let started = Instant::now();
        let wake = request(&mut queue, Pipes { text: Some(reader), ..Default::default() }, false);
        // Frames go on while the owner hasn't sent a byte: nothing lands, nothing waits.
        for _ in 0..100 {
            let mut raw = frame(vec![Event::PointerGone]);
            queue.hold(&mut raw);
            assert_eq!(queue.release(&mut raw), None);
            assert_eq!(raw.events, vec![Event::PointerGone]);
        }
        let waited = started.elapsed();
        assert!(waited < Duration::from_millis(500), "100 frames with a paste pending took {waited:?} (a blocking read waits {PASTE_TIMEOUT:?})");
        writer.write_all(b"late\r\ntext").unwrap();
        drop(writer);
        wake.recv_timeout(SOON).expect("the worker wakes the UI when the read is done");
        let mut raw = frame(Vec::new());
        assert_eq!(queue.release(&mut raw), Some(Landed { shift: false, text: true, files: None }));
        assert_eq!(raw.events, vec![Event::Paste("late\ntext".into())]);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn keys_typed_during_a_paste_follow_it() {
        let (reader, writer) = std::io::pipe().unwrap();
        let mut queue = Queue::default();
        // The frame of the paste key: what was typed before it goes straight through.
        let mut raw = frame(vec![key(Key::A)]);
        queue.hold(&mut raw);
        let wake = request(&mut queue, Pipes { text: Some(reader), ..Default::default() }, false);
        assert_eq!(queue.release(&mut raw), None);
        assert_eq!(raw.events, vec![key(Key::A)]);
        // Typed while it's read: held, though the pointer isn't.
        let mut raw = frame(vec![key(Key::Enter), Event::PointerGone, Event::Text("x".into())]);
        queue.hold(&mut raw);
        assert_eq!(queue.release(&mut raw), None);
        assert_eq!(raw.events, vec![Event::PointerGone]);
        (&writer).write_all(b"ls").unwrap();
        drop(writer);
        wake.recv_timeout(SOON).unwrap();
        // The paste lands alone, then the held keys in the order they were typed.
        let mut raw = frame(Vec::new());
        queue.hold(&mut raw);
        assert!(queue.release(&mut raw).is_some());
        assert_eq!(raw.events, vec![Event::Paste("ls".into())]);
        let mut raw = frame(vec![key(Key::B)]);
        queue.hold(&mut raw);
        assert_eq!(queue.release(&mut raw), None);
        assert_eq!(raw.events, vec![key(Key::Enter), Event::Text("x".into()), key(Key::B)]);
        assert_eq!(queue.len(), 0);
        // With nothing pending, input isn't touched.
        let mut raw = frame(vec![key(Key::C)]);
        queue.hold(&mut raw);
        assert_eq!(queue.release(&mut raw), None);
        assert_eq!(raw.events, vec![key(Key::C)]);
    }

    #[test]
    fn the_first_paste_lands_first() {
        let (slow, slow_writer) = std::io::pipe().unwrap();
        let mut queue = Queue::default();
        let first = request(&mut queue, Pipes { text: Some(slow), ..Default::default() }, false);
        let second = request(&mut queue, Pipes { text: sent(b"second"), ..Default::default() }, false);
        second.recv_timeout(SOON).unwrap();
        // The second is read, but waits for the first.
        let mut raw = frame(Vec::new());
        assert_eq!(queue.release(&mut raw), None);
        assert!(raw.events.is_empty());
        (&slow_writer).write_all(b"first").unwrap();
        drop(slow_writer);
        first.recv_timeout(SOON).unwrap();
        // One paste a frame.
        let mut raw = frame(Vec::new());
        assert!(queue.release(&mut raw).is_some());
        assert_eq!(raw.events, vec![Event::Paste("first".into())]);
        let mut raw = frame(Vec::new());
        assert!(queue.release(&mut raw).is_some());
        assert_eq!(raw.events, vec![Event::Paste("second".into())]);
    }

    #[test]
    fn held_input_keeps_the_modifiers_it_was_typed_under() {
        let mut queue = Queue::default();
        let wake = request(&mut queue, Pipes { text: sent(b"text"), ..Default::default() }, true);
        wake.recv_timeout(SOON).unwrap();
        // Shift is up again by the time the paste lands; Alt+x was typed meanwhile.
        let mut raw = RawInput { events: vec![Event::Text("x".into())], modifiers: Modifiers::ALT, ..Default::default() };
        queue.hold(&mut raw);
        assert_eq!(queue.release(&mut raw), Some(Landed { shift: true, text: true, files: None }));
        assert_eq!((raw.events, raw.modifiers), (vec![Event::Paste("text".into())], Modifiers::SHIFT));
        let mut raw = frame(vec![key(Key::A)]);
        queue.hold(&mut raw);
        assert_eq!(queue.release(&mut raw), None);
        assert_eq!((raw.events, raw.modifiers), (vec![Event::Text("x".into())], Modifiers::ALT));
        let mut raw = frame(Vec::new());
        assert_eq!(queue.release(&mut raw), None);
        assert_eq!((raw.events, raw.modifiers), (vec![key(Key::A)], Modifiers::NONE));
    }

    #[test]
    fn a_failed_or_timed_out_read_lands_with_nothing() {
        // The owner never closes its end.
        let (reader, _writer) = std::io::pipe().unwrap();
        let pipes = Pipes { text: Some(reader), ..Default::default() };
        assert_eq!(pipes.read_within(Duration::from_millis(20), Duration::from_millis(20)), Read::default());
        // Nothing to read at all, and a worker that went away without an answer.
        let mut queue = Queue::default();
        request(&mut queue, Pipes::default(), true).recv_timeout(SOON).unwrap();
        let (gone, read) = channel();
        drop(gone);
        queue.0.push_back(Held::Paste { shift: false, modifiers: Modifiers::NONE, read });
        for shift in [true, false] {
            let mut raw = frame(Vec::new());
            assert_eq!(queue.release(&mut raw), Some(Landed { shift, text: false, files: None }));
            assert!(raw.events.is_empty());
        }
    }

    #[test]
    fn copied_files_come_before_an_image() {
        // Only files that exist count.
        let file = std::env::temp_dir().join(format!("nex-paste-{}.txt", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let uris = format!("file://{}\r\n", file.display());
        let both = Pipes { text: sent(b"some text"), uris: sent(uris.as_bytes()), png: sent(b"png") };
        let read = both.read();
        let _ = std::fs::remove_file(&file);
        assert_eq!(read, Read { text: Some("some text".into()), files: Some(ClipboardFiles::Paths(vec![file])) });
        // A list with no local files in it (a browser's image URL) falls back to the image.
        let image = Pipes { text: None, uris: sent(b"https://example.com/a.png\r\n"), png: sent(b"png") };
        assert_eq!(image.read(), Read { text: None, files: Some(ClipboardFiles::Png(b"png".to_vec())) });
        assert_eq!(Pipes { png: sent(b""), ..Default::default() }.read(), Read::default());
    }
}
