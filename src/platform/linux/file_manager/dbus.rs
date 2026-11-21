//! Just enough of the D-Bus wire protocol to own a name on the session bus and answer method
//! calls on it: connect and authenticate, `Hello`, `RequestName`, then read calls and send
//! replies. Only the argument types nex's `FileManager1` methods use are read (`as`, `s`, `u`).
//!
//! The format is the one in the D-Bus specification: a fixed 12-byte header (endianness,
//! message type, flags, version, body length, serial), an array of `(byte, variant)` header
//! fields, padding to 8, then the body. Every value is aligned to its own size from the start
//! of the message.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;

pub const METHOD_CALL: u8 = 1;
const METHOD_RETURN: u8 = 2;
const ERROR: u8 = 3;
/// Header flag: the caller doesn't want a reply.
const NO_REPLY_EXPECTED: u8 = 0x1;

const PATH: u8 = 1;
const INTERFACE: u8 = 2;
const MEMBER: u8 = 3;
const ERROR_NAME: u8 = 4;
const REPLY_SERIAL: u8 = 5;
const DESTINATION: u8 = 6;
const SENDER: u8 = 7;
const SIGNATURE: u8 = 8;

/// `RequestName` replies.
pub const PRIMARY_OWNER: u32 = 1;
pub const ALREADY_OWNER: u32 = 4;
/// `RequestName` flag: fail rather than wait in line when someone else owns the name.
const DO_NOT_QUEUE: u32 = 0x4;

/// A message read off the bus, with the header fields nex looks at.
#[derive(Debug, Default)]
pub struct Message {
    pub kind: u8,
    pub flags: u8,
    pub serial: u32,
    pub path: String,
    pub interface: String,
    pub member: String,
    pub sender: String,
    pub signature: String,
    pub reply_serial: Option<u32>,
    big_endian: bool,
    body: Vec<u8>,
}

impl Message {
    pub fn wants_reply(&self) -> bool {
        self.kind == METHOD_CALL && self.flags & NO_REPLY_EXPECTED == 0
    }

    /// The body read as `as` followed by anything (the `FileManager1` methods are `(as, s)`).
    pub fn string_array(&self) -> io::Result<Vec<String>> {
        if !self.signature.starts_with("as") {
            return Err(bad(format!("expected (as…), got ({})", self.signature)));
        }
        let mut r = Reader { buf: &self.body, pos: 0, big_endian: self.big_endian };
        let len = r.u32()? as usize;
        let end = r.pos + len;
        let mut out = Vec::new();
        while r.pos < end {
            out.push(r.string()?);
        }
        Ok(out)
    }

    fn u32_body(&self) -> io::Result<u32> {
        if self.signature != "u" {
            return Err(bad(format!("expected (u), got ({})", self.signature)));
        }
        Reader { buf: &self.body, pos: 0, big_endian: self.big_endian }.u32()
    }
}

pub struct Connection {
    stream: BufReader<UnixStream>,
    serial: u32,
    /// Messages that arrived while `call` waited for its reply, handed out by `read` first.
    /// When the bus starts nex to answer a call, that call can arrive before `RequestName`'s
    /// reply.
    pending: VecDeque<Message>,
}

impl Connection {
    /// Connect to the session bus (`DBUS_SESSION_BUS_ADDRESS`, or `$XDG_RUNTIME_DIR/bus`) and
    /// say `Hello`.
    pub fn session() -> io::Result<Self> {
        let stream = connect(&session_address())?;
        let mut c = Connection { stream: BufReader::new(stream), serial: 0, pending: VecDeque::new() };
        c.authenticate()?;
        c.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "Hello", "", &[])?;
        Ok(c)
    }

    /// Ask for `name`, without queueing behind another owner. Returns the bus's answer
    /// ([`PRIMARY_OWNER`], [`ALREADY_OWNER`], or a code meaning someone else has it).
    pub fn request_name(&mut self, name: &str) -> io::Result<u32> {
        let mut body = Writer::default();
        body.string(name);
        body.u32(DO_NOT_QUEUE);
        let reply = self.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName", "su", &body.buf)?;
        reply.u32_body()
    }

    /// The next message, or `None` once the bus hangs up.
    pub fn read(&mut self) -> io::Result<Option<Message>> {
        if let Some(msg) = self.pending.pop_front() {
            return Ok(Some(msg));
        }
        self.read_wire()
    }

    fn read_wire(&mut self) -> io::Result<Option<Message>> {
        let mut fixed = [0u8; 16];
        match self.stream.read_exact(&mut fixed) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let big_endian = match fixed[0] {
            b'l' => false,
            b'B' => true,
            other => return Err(bad(format!("unknown byte order {other:#x}"))),
        };
        let word = |i: usize| {
            let b = [fixed[i], fixed[i + 1], fixed[i + 2], fixed[i + 3]];
            if big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
        };
        let (body_len, serial, fields_len) = (word(4) as usize, word(8), word(12) as usize);
        if body_len > 1 << 27 || fields_len > 1 << 26 {
            return Err(bad("message too large".into()));
        }
        let header_end = 16 + fields_len;
        let mut rest = vec![0u8; pad8(header_end) - 16 + body_len];
        self.stream.read_exact(&mut rest)?;
        let mut whole = fixed.to_vec();
        whole.extend_from_slice(&rest);

        let mut msg = Message { kind: fixed[1], flags: fixed[2], serial, big_endian, ..Default::default() };
        let mut r = Reader { buf: &whole[..header_end], pos: 16, big_endian };
        while r.pos < header_end {
            r.align(8)?;
            let code = r.byte()?;
            let sig = r.signature()?;
            match sig.as_str() {
                "s" | "o" => {
                    let value = r.string()?;
                    match code {
                        PATH => msg.path = value,
                        INTERFACE => msg.interface = value,
                        MEMBER => msg.member = value,
                        SENDER => msg.sender = value,
                        _ => {}
                    }
                }
                "g" => {
                    let value = r.signature()?;
                    if code == SIGNATURE {
                        msg.signature = value;
                    }
                }
                "u" => {
                    let value = r.u32()?;
                    if code == REPLY_SERIAL {
                        msg.reply_serial = Some(value);
                    }
                }
                // Only the types above appear in header fields today; stop rather than guess.
                _ => break,
            }
        }
        msg.body = whole.split_off(pad8(header_end));
        Ok(Some(msg))
    }

    /// Reply to `call` with `body` (of type `signature`).
    pub fn reply(&mut self, call: &Message, signature: &str, body: &[u8]) -> io::Result<()> {
        let mut fields = Fields::default();
        fields.u32(REPLY_SERIAL, call.serial);
        fields.string(DESTINATION, 's', &call.sender);
        if !signature.is_empty() {
            fields.signature(signature);
        }
        self.send(METHOD_RETURN, fields, body).map(drop)
    }

    /// Answer `call` with the D-Bus error `name` and a human-readable `text`.
    pub fn reply_error(&mut self, call: &Message, name: &str, text: &str) -> io::Result<()> {
        let mut fields = Fields::default();
        fields.u32(REPLY_SERIAL, call.serial);
        fields.string(DESTINATION, 's', &call.sender);
        fields.string(ERROR_NAME, 's', name);
        fields.signature("s");
        let mut body = Writer::default();
        body.string(text);
        self.send(ERROR, fields, &body.buf).map(drop)
    }

    /// Call a method and wait for its reply, keeping other messages for `read`.
    fn call(&mut self, dest: &str, path: &str, interface: &str, member: &str, signature: &str, body: &[u8]) -> io::Result<Message> {
        let mut fields = Fields::default();
        fields.string(PATH, 'o', path);
        fields.string(INTERFACE, 's', interface);
        fields.string(MEMBER, 's', member);
        fields.string(DESTINATION, 's', dest);
        if !signature.is_empty() {
            fields.signature(signature);
        }
        let serial = self.send(METHOD_CALL, fields, body)?;
        loop {
            let msg = self.read_wire()?.ok_or_else(|| bad("the bus hung up".into()))?;
            if msg.reply_serial != Some(serial) {
                self.pending.push_back(msg);
            } else {
                if msg.kind == ERROR {
                    let text = Reader { buf: &msg.body, pos: 0, big_endian: msg.big_endian }.string().unwrap_or_default();
                    return Err(bad(format!("{member}: {text}")));
                }
                return Ok(msg);
            }
        }
    }

    fn send(&mut self, kind: u8, fields: Fields, body: &[u8]) -> io::Result<u32> {
        self.serial += 1;
        let mut msg = Writer::default();
        msg.buf.extend_from_slice(&[b'l', kind, 0, 1]);
        msg.u32(body.len() as u32);
        msg.u32(self.serial);
        msg.u32(fields.0.buf.len() as u32);
        msg.buf.extend_from_slice(&fields.0.buf);
        msg.align(8);
        msg.buf.extend_from_slice(body);
        self.stream.get_mut().write_all(&msg.buf)?;
        Ok(self.serial)
    }

    /// `AUTH EXTERNAL` with our uid, the one mechanism every bus on Linux accepts.
    fn authenticate(&mut self) -> io::Result<()> {
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() }.to_string();
        let hex: String = uid.bytes().map(|b| format!("{b:02x}")).collect();
        self.stream.get_mut().write_all(format!("\0AUTH EXTERNAL {hex}\r\n").as_bytes())?;
        let mut line = String::new();
        self.stream.read_line(&mut line)?;
        if !line.starts_with("OK ") {
            return Err(bad(format!("the bus refused authentication: {}", line.trim())));
        }
        self.stream.get_mut().write_all(b"BEGIN\r\n")
    }
}

/// The session bus address: the variable when set, else the standard per-user socket.
fn session_address() -> String {
    std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().filter(|a| !a.is_empty()).unwrap_or_else(|| {
        let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
            // SAFETY: getuid has no preconditions and cannot fail.
            format!("/run/user/{}", unsafe { libc::getuid() })
        });
        format!("unix:path={runtime}/bus")
    })
}

/// Connect to the first `unix:path=` or `unix:abstract=` entry of a bus address.
fn connect(address: &str) -> io::Result<UnixStream> {
    for entry in address.split(';') {
        let Some(params) = entry.strip_prefix("unix:") else { continue };
        for kv in params.split(',') {
            if let Some(path) = kv.strip_prefix("path=") {
                return UnixStream::connect(unescape(path));
            }
            if let Some(name) = kv.strip_prefix("abstract=") {
                use std::os::linux::net::SocketAddrExt;
                let addr = std::os::unix::net::SocketAddr::from_abstract_name(unescape(name).as_bytes())?;
                return UnixStream::connect_addr(&addr);
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, format!("no unix socket in bus address `{address}`")))
}

/// Bus addresses escape bytes as `%xx`.
fn unescape(s: &str) -> String {
    crate::file_explorer::fs::ops::percent_decode(s)
}

fn bad(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn pad8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    big_endian: bool,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len()).ok_or_else(|| bad("message ends early".into()))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn align(&mut self, n: usize) -> io::Result<()> {
        let to = self.pos.div_ceil(n) * n;
        self.take(to - self.pos).map(drop)
    }

    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        self.align(4)?;
        let big_endian = self.big_endian;
        let b: [u8; 4] = self.take(4)?.try_into().unwrap();
        Ok(if big_endian { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }

    /// `s` / `o`: u32 length, bytes, a trailing nul.
    fn string(&mut self) -> io::Result<String> {
        let len = self.u32()? as usize;
        let s = String::from_utf8_lossy(self.take(len)?).into_owned();
        self.take(1)?;
        Ok(s)
    }

    /// `g`: byte length, bytes, a trailing nul.
    fn signature(&mut self) -> io::Result<String> {
        let len = self.byte()? as usize;
        let s = String::from_utf8_lossy(self.take(len)?).into_owned();
        self.take(1)?;
        Ok(s)
    }
}

/// Builds a little-endian message body.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    fn align(&mut self, n: usize) {
        while !self.buf.len().is_multiple_of(n) {
            self.buf.push(0);
        }
    }

    pub fn u32(&mut self, v: u32) {
        self.align(4);
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn string(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }

    fn signature(&mut self, s: &str) {
        self.buf.push(s.len() as u8);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
    }
}

/// Header fields, written as they go. The array starts at offset 16 of the message, which is
/// 8-aligned, so aligning inside this buffer matches aligning in the message.
#[derive(Default)]
struct Fields(Writer);

impl Fields {
    fn string(&mut self, code: u8, kind: char, value: &str) {
        self.0.align(8);
        self.0.buf.push(code);
        self.0.signature(&kind.to_string());
        self.0.string(value);
    }

    fn u32(&mut self, code: u8, value: u32) {
        self.0.align(8);
        self.0.buf.push(code);
        self.0.signature("u");
        self.0.u32(value);
    }

    fn signature(&mut self, value: &str) {
        self.0.align(8);
        self.0.buf.push(SIGNATURE);
        self.0.signature("g");
        self.0.signature(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A method call as a bus would deliver it, for `read` to parse back.
    fn call_bytes(member: &str, uris: &[&str]) -> Vec<u8> {
        let mut fields = Fields::default();
        fields.string(PATH, 'o', "/org/freedesktop/FileManager1");
        fields.string(INTERFACE, 's', "org.freedesktop.FileManager1");
        fields.string(MEMBER, 's', member);
        fields.string(SENDER, 's', ":1.42");
        fields.signature("ass");
        let mut items = Writer::default();
        for u in uris {
            items.string(u);
        }
        let mut body = Writer::default();
        body.u32(items.buf.len() as u32);
        body.buf.extend_from_slice(&items.buf);
        body.string("startup-id");
        let mut msg = Writer::default();
        msg.buf.extend_from_slice(&[b'l', METHOD_CALL, 0, 1]);
        msg.u32(body.buf.len() as u32);
        msg.u32(7);
        msg.u32(fields.0.buf.len() as u32);
        msg.buf.extend_from_slice(&fields.0.buf);
        msg.align(8);
        msg.buf.extend_from_slice(&body.buf);
        msg.buf
    }

    #[test]
    fn reads_a_show_items_call() {
        let (mut a, b) = UnixStream::pair().unwrap();
        a.write_all(&call_bytes("ShowItems", &["file:///tmp/a%20b.txt", "file:///etc/hosts"])).unwrap();
        drop(a);
        let mut c = Connection { stream: BufReader::new(b), serial: 0, pending: VecDeque::new() };
        let msg = c.read().unwrap().unwrap();
        assert_eq!(msg.kind, METHOD_CALL);
        assert_eq!(msg.serial, 7);
        assert_eq!(msg.member, "ShowItems");
        assert_eq!(msg.interface, "org.freedesktop.FileManager1");
        assert_eq!(msg.path, "/org/freedesktop/FileManager1");
        assert_eq!(msg.sender, ":1.42");
        assert_eq!(msg.signature, "ass");
        assert!(msg.wants_reply());
        assert_eq!(msg.string_array().unwrap(), vec!["file:///tmp/a%20b.txt", "file:///etc/hosts"]);
        assert!(c.read().unwrap().is_none());
    }

    #[test]
    fn reply_parses_back() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut sender = Connection { stream: BufReader::new(a), serial: 0, pending: VecDeque::new() };
        let call = Message { kind: METHOD_CALL, serial: 9, sender: ":1.5".into(), ..Default::default() };
        sender.reply(&call, "", &[]).unwrap();
        sender.reply_error(&call, "org.freedesktop.DBus.Error.UnknownMethod", "nope").unwrap();
        let mut receiver = Connection { stream: BufReader::new(b), serial: 0, pending: VecDeque::new() };
        let ok = receiver.read().unwrap().unwrap();
        assert_eq!((ok.kind, ok.reply_serial), (METHOD_RETURN, Some(9)));
        let err = receiver.read().unwrap().unwrap();
        assert_eq!((err.kind, err.reply_serial, err.signature.as_str()), (ERROR, Some(9), "s"));
    }

    /// A reply to our call `serial`, carrying one `u`.
    fn u32_reply_bytes(serial: u32, value: u32) -> Vec<u8> {
        let mut fields = Fields::default();
        fields.u32(REPLY_SERIAL, serial);
        fields.signature("u");
        let mut msg = Writer::default();
        msg.buf.extend_from_slice(&[b'l', METHOD_RETURN, 0, 1]);
        msg.u32(4);
        msg.u32(99);
        msg.u32(fields.0.buf.len() as u32);
        msg.buf.extend_from_slice(&fields.0.buf);
        msg.align(8);
        msg.u32(value);
        msg.buf
    }

    #[test]
    fn calls_arriving_before_request_name_returns_are_kept() {
        let (mut bus, ours) = UnixStream::pair().unwrap();
        // Bus activation: the waiting call is delivered before RequestName's reply.
        bus.write_all(&call_bytes("ShowItems", &["file:///tmp/x"])).unwrap();
        bus.write_all(&u32_reply_bytes(1, PRIMARY_OWNER)).unwrap();
        let mut c = Connection { stream: BufReader::new(ours), serial: 0, pending: VecDeque::new() };
        assert_eq!(c.request_name("org.example.Test").unwrap(), PRIMARY_OWNER);
        assert_eq!(c.read().unwrap().unwrap().member, "ShowItems");
    }

    #[test]
    fn truncated_bodies_are_errors() {
        let msg = Message { signature: "ass".into(), body: vec![40, 0, 0, 0, 3, 0], ..Default::default() };
        assert!(msg.string_array().is_err());
    }
}
