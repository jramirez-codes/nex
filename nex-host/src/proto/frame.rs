//! The frames an attached client sends the daemon: keys and resizes.

/// Keys typed into an attached session.
pub const FRAME_INPUT: u8 = 0;
/// The attached pane's new size: columns then rows, each a big-endian `u16`.
pub const FRAME_RESIZE: u8 = 1;
/// The largest frame either side accepts.
pub const FRAME_MAX: usize = 1 << 20;

/// A frame from an attached client: its kind, the payload's length (big-endian `u32`), then
/// the payload.
pub fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// A resize frame.
pub fn resize_frame(cols: u16, rows: u16) -> Vec<u8> {
    let mut p = cols.to_be_bytes().to_vec();
    p.extend_from_slice(&rows.to_be_bytes());
    frame(FRAME_RESIZE, &p)
}
