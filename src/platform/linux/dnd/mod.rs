//! Linux backend for `platform::Dnd`: drag and drop with other Wayland clients (another nex
//! window, a file manager, a browser).
//!
//! winit 0.30 has no drag and drop on Wayland (only on X11), so nex talks to the compositor
//! itself. It shares winit's connection but has its own event queue, dispatched on a
//! background thread:
//!
//! - **Out:** an explorer drag that leaves the window becomes a compositor drag of
//!   `text/uri-list` (`start_drag`), with the dragged icon on a surface of its own so it
//!   keeps following the pointer outside the window. A drag must name the press (or touch) that started it, so
//!   the thread also listens to the seat's pointer and touch for their serials.
//! - **In:** a drag over the window is reported to egui the way winit reports one on X11:
//!   hovered files and the pointer moving with it, then dropped files. The app's existing drop
//!   handling takes it from there (a terminal types the paths, an explorer moves the files, or
//!   copies them when the drag's source only allows a copy).
//! - **Move vs copy:** drags out offer both; drags in prefer a move. As with Thunar or
//!   Nautilus, the side that receives a move moves the files itself; the source just re-reads
//!   its listings once the drag is finished.
//! - **First device:** Hyprland sends drags and the clipboard only to the first data device a
//!   client made, and egui's clipboard (smithay-clipboard) makes one as the window opens. So
//!   `main` creates this one first (`prepare`, on its own event loop, before eframe opens the
//!   window), and with it nex reads the clipboard itself: a paste key (see `note_paste_key`)
//!   becomes a paste of the selection this device was offered.
//! - **Touch:** sway keeps a finger's drag on the window it started in, so this window passes
//!   it on to the nex under the finger itself (see `relay`).

mod relay;

use std::io::{Read, Write};
use std::os::fd::{AsFd, FromRawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{DroppedFile, Event, HoveredFile, PointerButton, Pos2, RawInput, TouchDeviceId, TouchId, TouchPhase};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::{Backend, ObjectId, WaylandError};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::protocol::wl_touch::{self, WlTouch};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

use crate::file_explorer::fs::ops::parse_paths;
use crate::platform::{ClipboardFiles, DragImage, dnd_log as log};

macro_rules! trace {
    ($($t:tt)*) => { log(format_args!($($t)*)) };
}

const URI_LIST: &str = "text/uri-list";
const TEXT: &str = "text/plain;charset=utf-8";
const PNG: &str = "image/png";
/// Text types a pasted selection may come in, best first.
const PASTE_TYPES: [&str; 5] = [TEXT, "UTF8_STRING", "text/plain", "TEXT", "STRING"];
/// How long a paste waits for the app holding the clipboard to send it.
const PASTE_TIMEOUT: Duration = Duration::from_secs(2);
/// The same for an image, which can be megabytes.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(8);
/// Image types a drop with no files (an image dragged out of a browser) is read as, best
/// first, with the extension it's saved under. Other `image/*` types come after these.
const DROP_IMAGE_TYPES: [(&str, &str); 4] = [(PNG, "png"), ("image/jpeg", "jpg"), ("image/gif", "gif"), ("image/webp", "webp")];

/// A paste key was pressed in the window since the last frame (set by `note_paste_key`).
static PASTE_KEY: AtomicBool = AtomicBool::new(false);

/// The data device made by `prepare`, before eframe opened the window, waiting for `Dnd::new`.
static EARLY: Mutex<Option<Early>> = Mutex::new(None);

/// A connection with its data device made, its events queued until the thread starts.
struct Early {
    /// The `wl_display` it was made on, to check `Dnd::new` gets the same one.
    display: usize,
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
}

impl Early {
    fn connect(display: *mut std::ffi::c_void) -> Result<Self, String> {
        // SAFETY: the pointer comes from winit's live event loop, which outlives the app (and
        // so this connection and its thread, which only use it through libwayland).
        let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
        let mut queue: EventQueue<State> = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = State {
            // Replaced by the app's context before the thread starts.
            ctx: egui::Context::default(),
            conn: conn.clone(),
            shared: Arc::new(Mutex::new(Shared::default())),
            seat: None,
            manager: None,
            compositor: None,
            shm: None,
            pointer: None,
            touch: None,
            offer: None,
            at: None,
            layout: None,
            relaying_to: None,
        };
        conn.display().get_registry(&qh, ());
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        let (Some(seat), Some(manager)) = (state.seat.clone(), state.manager.clone()) else {
            return Err("no seat or data device manager".into());
        };
        let device = manager.get_data_device(&seat, &qh, ());
        conn.flush().map_err(|e| e.to_string())?;
        trace!("data device made");
        Ok(Self { display: display as usize, conn, queue, state, manager, device })
    }
}

/// `blocking_dispatch` in a loop, that also wakes when `stop` is readable or closed, and then
/// returns, leaving the socket alone. Returns too when the connection fails.
fn dispatch_until_stopped(queue: &mut EventQueue<State>, state: &mut State, stop: &std::io::PipeReader) {
    use std::os::fd::AsRawFd;
    let would_block = |e: &WaylandError| matches!(e, WaylandError::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock);
    loop {
        if queue.dispatch_pending(state).is_err() {
            return;
        }
        match queue.flush() {
            Err(e) if !would_block(&e) => return,
            _ => {}
        }
        // `None`: events arrived for this queue meanwhile; dispatch them first.
        let Some(guard) = queue.prepare_read() else { continue };
        let mut fds = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: stop.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: two valid pollfds for open fds.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue; // Dropping `guard` cancels the read.
            }
            return;
        }
        if fds[1].revents != 0 {
            trace!("dispatch thread stopping");
            return;
        }
        if fds[0].revents != 0 {
            match guard.read() {
                Err(e) if !would_block(&e) => return,
                _ => {}
            }
        }
    }
}

/// Read `reader` to its end, giving up (`None`) after `timeout`.
fn read_until_closed(mut reader: std::io::PipeReader, timeout: Duration) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            trace!("paste: timed out");
            return None;
        }
        let mut pfd = libc::pollfd { fd: reader.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd for an open fd.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().clamp(1, i32::MAX as u128) as i32) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if ready == 0 {
            continue;
        }
        match reader.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// What the background thread has seen, for the UI thread.
#[derive(Default)]
struct Shared {
    /// Serial of the latest button press or touch down on this client.
    press_serial: Option<u32>,
    /// A drag of files is over the window, at this surface-local position.
    hover: Option<(f64, f64)>,
    /// Files dropped on the window, with where, and whether the drag was a move.
    dropped: Vec<(Vec<PathBuf>, (f64, f64), bool)>,
    /// The window's size in surface coordinates, once drawn.
    size: Option<(f64, f64)>,
    /// The clipboard's offer to this device (what a paste reads).
    selection: Option<WlDataOffer>,
}

impl Shared {
    /// `at` (surface coordinates) is on the window, or its size isn't known yet.
    fn on_window(&self, (x, y): (f64, f64)) -> bool {
        self.size.is_none_or(|(w, h)| x >= 0.0 && y >= 0.0 && x < w && y < h)
    }
}

pub struct Dnd {
    conn: Connection,
    qh: QueueHandle<State>,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    surface: WlSurface,
    /// For drag icons (`None` if the compositor lacks them: drags go out without one).
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    shared: Arc<Mutex<Shared>>,
    /// An incoming drag was hovering last frame (so egui hears when it leaves).
    hovering: bool,
    /// Accepts touch drags relayed by other nex windows.
    _relay: Option<relay::Listener>,
    /// A drag out of this window was handed to the compositor at this position. The compositor
    /// keeps the release of the finger or button that started it, so the app is told the press
    /// ended (or it would think the finger is still down: a long press, a stuck drag).
    handed_off: Option<Pos2>,
    /// The touch the app saw last, still down (to end it after a hand-off).
    touch_down: Option<(TouchDeviceId, TouchId, Pos2)>,
    /// The files fed to egui last were dropped as a move (not a copy).
    drop_moves: bool,
    /// Dropped to tell the thread to stop (see [`Drop`] for `Dnd`).
    stop: Option<std::io::PipeWriter>,
    /// Disconnects when the thread has stopped.
    stopped: std::sync::mpsc::Receiver<()>,
}

/// Stop the thread before the Wayland connection goes. `main` drops the event loop, which
/// disconnects from the display, after the app (and this) is dropped; a thread still
/// reading the display then would use freed memory and crash nex as it quits.
impl Drop for Dnd {
    fn drop(&mut self) {
        self.stop.take();
        // Waits for the thread, but never holds up quitting for long.
        let _ = self.stopped.recv_timeout(Duration::from_secs(1));
    }
}

impl Dnd {
    /// Set up on the window's Wayland connection; `None` on X11 or if the compositor lacks
    /// drag and drop.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) =
            (cc.display_handle().ok()?.as_raw(), cc.window_handle().ok()?.as_raw())
        else {
            return None;
        };
        match Self::connect(cc.egui_ctx.clone(), display.display.as_ptr(), window.surface.as_ptr()) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("nex: drag and drop with other windows is unavailable: {e}");
                None
            }
        }
    }

    /// Make the data device on `event_loop`'s connection now, before eframe opens the window
    /// (and egui's clipboard makes its own device): see "First device" above. `new` picks it up.
    pub fn prepare(event_loop: &impl HasDisplayHandle) {
        let Ok(handle) = event_loop.display_handle() else { return };
        let RawDisplayHandle::Wayland(display) = handle.as_raw() else { return };
        match Early::connect(display.display.as_ptr()) {
            Ok(early) => *EARLY.lock().unwrap() = Some(early),
            Err(e) => trace!("prepare: {e}"),
        }
    }

    /// A paste key went down in the window. egui only pastes what its own clipboard holds,
    /// which on Hyprland is never offered anything, so the next frame pastes the selection
    /// this device holds instead.
    pub fn note_paste_key() {
        PASTE_KEY.store(true, Ordering::Relaxed);
    }

    fn connect(ctx: egui::Context, display: *mut std::ffi::c_void, surface: *mut std::ffi::c_void) -> Result<Self, String> {
        let early = EARLY.lock().unwrap().take().filter(|e| e.display == display as usize);
        let Early { conn, mut queue, mut state, manager, device, .. } = match early {
            Some(e) => e,
            None => {
                trace!("no data device made before the window: making one now");
                Early::connect(display)?
            }
        };
        // SAFETY: the pointer comes from the live winit window, which outlives the app (and
        // so this struct and its thread, which only use it through libwayland).
        let surface_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.map_err(|e| e.to_string())?;
        let surface = WlSurface::from_id(&conn, surface_id).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let shared = state.shared.clone();
        let _relay = relay::listen(shared.clone(), ctx.clone());
        state.ctx = ctx;
        let (compositor, shm) = (state.compositor.clone(), state.shm.clone());
        conn.flush().map_err(|e| e.to_string())?;

        let (stop_rx, stop) = std::io::pipe().map_err(|e| e.to_string())?;
        let (done, stopped) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("nex-wl-dnd".into())
            .spawn(move || {
                dispatch_until_stopped(&mut queue, &mut state, &stop_rx);
                // Let go of the connection's objects while the display is still there.
                drop((queue, state));
                drop(done);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            conn,
            qh,
            manager,
            device,
            surface,
            compositor,
            shm,
            shared,
            hovering: false,
            _relay,
            handed_off: None,
            touch_down: None,
            drop_moves: false,
            stop: Some(stop),
            stopped,
        })
    }

    /// The clipboard's text, if it holds any: read from the app holding it, waiting at most
    /// `PASTE_TIMEOUT`.
    fn clipboard_text(&self) -> Option<String> {
        let types = self.clipboard_types()?;
        let Some(mime) = PASTE_TYPES.into_iter().find(|t| types.iter().any(|x| x == t)) else {
            trace!("paste: no text in the clipboard ({types:?})");
            return None;
        };
        let bytes = self.read_clipboard(mime, PASTE_TIMEOUT)?;
        let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        trace!("paste: {} bytes as {mime}", text.len());
        (!text.is_empty()).then_some(text)
    }

    /// Copied files (a file manager's copy) or an image (a screenshot), if the clipboard
    /// holds either.
    pub fn clipboard_files(&self) -> Option<ClipboardFiles> {
        let types = self.clipboard_types()?;
        let has = |t: &str| types.iter().any(|x| x == t);
        if has(URI_LIST) {
            let paths = parse_paths(&String::from_utf8_lossy(&self.read_clipboard(URI_LIST, PASTE_TIMEOUT)?));
            if !paths.is_empty() {
                return Some(ClipboardFiles::Paths(paths));
            }
        }
        if has(PNG) {
            let png = self.read_clipboard(PNG, IMAGE_TIMEOUT).filter(|b| !b.is_empty())?;
            trace!("paste: {} bytes of {PNG}", png.len());
            return Some(ClipboardFiles::Png(png));
        }
        None
    }

    /// The types the clipboard's current owner offers.
    fn clipboard_types(&self) -> Option<Vec<String>> {
        let offer = self.shared.lock().unwrap().selection.clone()?;
        let types = offer.data::<OfferData>()?.types.lock().unwrap().clone();
        Some(types)
    }

    /// The clipboard's contents as `mime`, waiting at most `timeout` for its owner.
    fn read_clipboard(&self, mime: &str, timeout: Duration) -> Option<Vec<u8>> {
        let offer = self.shared.lock().unwrap().selection.clone()?;
        let (reader, writer) = std::io::pipe().ok()?;
        offer.receive(mime.into(), writer.as_fd());
        drop(writer);
        self.conn.flush().ok()?;
        read_until_closed(reader, timeout)
    }

    /// Hand `paths` to the compositor as a drag out of this window, continuing the press that
    /// is dragging them now (at `at`, in egui points), showing `image` under the pointer.
    /// Returns false if there's no press to continue.
    pub fn start_drag(&mut self, paths: &[PathBuf], at: Pos2, image: Option<DragImage>) -> bool {
        let Some(serial) = self.shared.lock().unwrap().press_serial else {
            trace!("start_drag: no press serial yet");
            return false;
        };
        trace!("start_drag: {} path(s), serial {serial}", paths.len());
        let data = SourceData {
            uri_list: paths.iter().map(|p| file_uri(p) + "\r\n").collect(),
            text: paths.iter().map(|p| crate::platform::shell_quote(p)).collect::<Vec<_>>().join(" "),
            icon: Mutex::new(None),
        };
        let source = self.manager.create_data_source(&self.qh, data);
        source.offer(URI_LIST.into());
        source.offer(TEXT.into());
        if source.version() >= 3 {
            // The receiver picks: a file manager (or another nex) moves the files by default.
            source.set_actions(DndAction::Copy | DndAction::Move);
        }
        let icon = image.and_then(|img| self.icon_surface(&img).map(|i| (i, img.hotspot)));
        self.device.start_drag(Some(&source), &self.surface, icon.as_ref().map(|(i, _)| &i.surface), serial);
        if let Some((icon, [hx, hy])) = icon {
            // The icon's top-left starts at the pointer: move it so the pointer is where the
            // file was grabbed.
            if icon.surface.version() >= 5 {
                icon.surface.offset(-hx, -hy);
                icon.surface.attach(Some(&icon.buffer), 0, 0);
            } else {
                icon.surface.attach(Some(&icon.buffer), -hx, -hy);
            }
            icon.surface.damage(0, 0, i32::MAX, i32::MAX);
            icon.surface.commit();
            // Kept until the drag ends (see the source's events).
            *source.data::<SourceData>().expect("source data").icon.lock().unwrap() = Some(icon);
        }
        self.handed_off = Some(at);
        self.conn.flush().is_ok()
    }

    /// A surface showing `img`, for a drag icon.
    fn icon_surface(&self, img: &DragImage) -> Option<DragIcon> {
        let (compositor, shm) = (self.compositor.as_ref()?, self.shm.as_ref()?);
        let [w, h] = img.size;
        if w == 0 || h == 0 {
            return None;
        }
        // ARGB8888 is premultiplied, little-endian: B, G, R, A in memory.
        let bytes: Vec<u8> = img.pixels.iter().flat_map(|&[r, g, b, a]| [b, g, r, a]).collect();
        // SAFETY: plain syscall; the name is a valid C string and the fd is checked.
        let fd = unsafe { libc::memfd_create(c"nex-drag-icon".as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            trace!("drag icon: memfd_create failed");
            return None;
        }
        // SAFETY: `fd` was just created and is owned by nothing else.
        let mut file = std::fs::File::from(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) });
        file.write_all(&bytes).ok()?;
        let len = i32::try_from(bytes.len()).ok()?;
        let pool = shm.create_pool(file.as_fd(), len, &self.qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, w as i32 * 4, wl_shm::Format::Argb8888, &self.qh, ());
        let surface = compositor.create_surface(&self.qh, ());
        if surface.version() >= 3 {
            surface.set_buffer_scale(img.scale.max(1));
        }
        trace!("drag icon: {w}x{h} px, scale {}, hotspot {:?}", img.scale, img.hotspot);
        Some(DragIcon { surface, buffer, pool })
    }

    /// Files from another window were dropped on this one: bring it to the front (keyboard
    /// focus), as a drop within the window focuses the pane it landed on.
    pub fn focus_window(&self) {
        std::thread::spawn(relay::focus_this_window);
    }

    /// The files dropped this frame (egui's `dropped_files`) came from a drag that moves them;
    /// otherwise they're copied.
    pub fn drop_moves(&self) -> bool {
        self.drop_moves
    }

    /// Before each frame: report an incoming drag to egui as winit does on X11, and a paste
    /// key egui couldn't paste for.
    pub fn feed(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
        if PASTE_KEY.swap(false, Ordering::Relaxed) && !raw.events.iter().any(|e| matches!(e, Event::Paste(_)))
            && let Some(text) = self.clipboard_text() {
                raw.events.push(Event::Paste(text));
            }
        let (hover, dropped) = {
            let mut s = self.shared.lock().unwrap();
            let size = ctx.content_rect().size() * ctx.zoom_factor();
            s.size = Some((size.x as f64, size.y as f64));
            (s.hover, std::mem::take(&mut s.dropped))
        };
        // Surface coordinates are logical pixels; egui points are those over the zoom.
        let to_pos = |(x, y): (f64, f64)| Pos2::new(x as f32, y as f32) / ctx.zoom_factor();
        if !dropped.is_empty() {
            trace!("delivering {} drop(s) to the app", dropped.len());
        }
        // End the press that a drag out of this window took with it.
        for e in &raw.events {
            if let Event::Touch { device_id, id, phase, pos, .. } = *e {
                self.touch_down = matches!(phase, TouchPhase::Start | TouchPhase::Move).then_some((device_id, id, pos));
            }
        }
        if let Some(at) = self.handed_off.take() {
            trace!("ending the press that started the drag");
            if let Some((device_id, id, pos)) = self.touch_down.take() {
                raw.events.push(Event::Touch { device_id, id, phase: TouchPhase::Cancel, pos, force: None });
            }
            raw.events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: raw.modifiers });
            raw.events.push(Event::PointerGone);
        }
        for (paths, at, moves) in dropped {
            self.drop_moves = moves;
            raw.events.push(Event::PointerMoved(to_pos(at)));
            raw.dropped_files.extend(paths.into_iter().map(|p| DroppedFile { path: Some(p), ..Default::default() }));
        }
        if let Some(at) = hover {
            raw.events.push(Event::PointerMoved(to_pos(at)));
            raw.hovered_files.push(HoveredFile::default());
        } else if self.hovering {
            raw.events.push(Event::PointerGone);
        }
        self.hovering = hover.is_some();
    }
}

/// What a drag out of this window offers.
struct SourceData {
    uri_list: String,
    text: String,
    /// The icon following the pointer, destroyed when the drag ends.
    icon: Mutex<Option<DragIcon>>,
}

/// The surface (and its memory) showing a drag's icon.
struct DragIcon {
    surface: WlSurface,
    buffer: WlBuffer,
    pool: WlShmPool,
}

impl DragIcon {
    fn destroy(self) {
        self.surface.destroy();
        self.buffer.destroy();
        self.pool.destroy();
    }
}

/// What an offered drag (or selection) carries.
#[derive(Default)]
struct OfferData {
    uris: AtomicBool,
    /// Every type it's offered as.
    types: Mutex<Vec<String>>,
    /// The action the compositor settled on for the drag (`DndAction` bits).
    action: AtomicU32,
}

impl OfferData {
    fn moves(&self) -> bool {
        self.action.load(Ordering::Relaxed) == DndAction::Move.bits()
    }

    /// The image type to read a drop as when it carries no local files, with the extension
    /// to save it under. A browser's image drag offers the image's URL as `text/uri-list`
    /// (an `https://` one, not a file) and the image itself as `image/png` and the like.
    fn image_type(&self) -> Option<(String, String)> {
        let types = self.types.lock().unwrap();
        for (mime, ext) in DROP_IMAGE_TYPES {
            if types.iter().any(|t| t == mime) {
                return Some((mime.to_string(), ext.to_string()));
            }
        }
        let other = types.iter().find(|t| t.starts_with("image/") && !t.contains(';'))?;
        let ext = other["image/".len()..].trim_start_matches("x-").split('+').next().unwrap_or("img").to_string();
        Some((other.clone(), ext))
    }
}

/// The background thread's state.
struct State {
    ctx: egui::Context,
    conn: Connection,
    shared: Arc<Mutex<Shared>>,
    seat: Option<WlSeat>,
    manager: Option<WlDataDeviceManager>,
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    pointer: Option<WlPointer>,
    touch: Option<WlTouch>,
    /// The drag over the window now, if it carries files (or an image).
    offer: Option<WlDataOffer>,
    /// Where the drag over this window's surface is now (possibly off the window).
    at: Option<(f64, f64)>,
    /// The other nex windows, looked up once per drag the first time it goes off the window.
    layout: Option<Option<relay::Layout>>,
    /// The nex window a drag off this one is being relayed to.
    relaying_to: Option<u32>,
}

/// Where dropped files go.
enum Dest {
    /// This window, at this position.
    Here((f64, f64)),
    /// Another nex window (process id), at this position in it.
    Relay(u32, (f64, f64)),
}

impl State {
    fn set_hover(&self, hover: Option<(f64, f64)>) {
        self.shared.lock().unwrap().hover = hover;
        self.ctx.request_repaint();
    }

    /// The drag moved to `at`. On the window it hovers here; off it (a finger's drag, which
    /// sway leaves with this window), it's relayed to the nex window under it, if any.
    fn track(&mut self, at: (f64, f64)) {
        self.at = Some(at);
        if self.shared.lock().unwrap().on_window(at) {
            self.stop_relaying();
            self.set_hover(Some(at));
            return;
        }
        self.set_hover(None);
        let target = self.relay_target(at);
        if target.map(|(pid, _)| pid) != self.relaying_to {
            self.stop_relaying();
        }
        if let Some((pid, (x, y))) = target {
            relay::send(pid, &format!("hover {x} {y}\n"));
            self.relaying_to = Some(pid);
        }
    }

    fn relay_target(&mut self, at: (f64, f64)) -> Option<(u32, (f64, f64))> {
        self.layout.get_or_insert_with(relay::Layout::query).as_ref()?.target(at)
    }

    fn stop_relaying(&mut self) {
        if let Some(pid) = self.relaying_to.take() {
            relay::send(pid, "leave\n");
        }
    }

    /// The drag is over (dropped or gone).
    fn end_drag(&mut self) {
        self.stop_relaying();
        self.at = None;
        self.layout = None;
        self.set_hover(None);
    }

    /// Read the dropped file list on another thread (the drag may come from this very
    /// process, whose source answers on this thread), then deliver it.
    fn receive_drop(&mut self, offer: WlDataOffer, dest: Dest) {
        // Before version 3 there are no actions: a drop is a copy.
        let moves = offer.version() >= 3 && offer.data::<OfferData>().is_some_and(OfferData::moves);
        trace!("receiving drop (offer v{}), move: {moves}", offer.version());
        let data = offer.data::<OfferData>();
        let uris = data.is_some_and(|d| d.uris.load(Ordering::Relaxed));
        let image = data.and_then(OfferData::image_type);
        let reader = if uris {
            let Ok((reader, writer)) = std::io::pipe() else { return };
            offer.receive(URI_LIST.into(), writer.as_fd());
            drop(writer);
            let _ = self.conn.flush();
            Some(reader)
        } else {
            None
        };
        let (conn, shared, ctx) = (self.conn.clone(), self.shared.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut reader) = reader {
                let _ = reader.read_to_string(&mut text);
            }
            let mut paths = parse_paths(&text);
            trace!("received {} bytes, {} existing path(s)", text.len(), paths.len());
            // No local files (a browser's image, whose URI is a web URL): save the image
            // itself and drop that file, so a terminal types its path as for any file.
            if paths.is_empty()
                && let Some((mime, ext)) = image
                    && let Some(path) = receive_image(&conn, &offer, &mime, &ext) {
                        text = file_uri(&path);
                        paths = vec![path];
                    }
            if offer.version() >= 3 {
                offer.finish();
            }
            offer.destroy();
            let _ = conn.flush();
            match dest {
                Dest::Here(at) if !paths.is_empty() => {
                    shared.lock().unwrap().dropped.push((paths, at, moves));
                    ctx.request_repaint();
                }
                Dest::Relay(pid, (x, y)) => {
                    let verb = if moves { "move" } else { "drop" };
                    relay::send(pid, &format!("{verb} {x} {y}\n{text}"));
                }
                Dest::Here(_) => {}
            }
        });
    }
}

/// Read a drop's image as `mime` and save it to a temp file (see `remote::save_image`).
fn receive_image(conn: &Connection, offer: &WlDataOffer, mime: &str, ext: &str) -> Option<PathBuf> {
    let (reader, writer) = std::io::pipe().ok()?;
    offer.receive(mime.into(), writer.as_fd());
    drop(writer);
    conn.flush().ok()?;
    let bytes = read_until_closed(reader, IMAGE_TIMEOUT).filter(|b| !b.is_empty())?;
    trace!("received {} bytes of {mime}", bytes.len());
    match crate::remote::save_image(&bytes, "dropped", ext) {
        Ok(path) => Some(path),
        Err(e) => {
            trace!("couldn't save the dropped image: {e}");
            None
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(state: &mut Self, registry: &WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => state.seat = Some(registry.bind(name, version.min(5), qh, ())),
                "wl_data_device_manager" if state.manager.is_none() => state.manager = Some(registry.bind(name, version.min(3), qh, ())),
                "wl_compositor" if state.compositor.is_none() => state.compositor = Some(registry.bind(name, version.min(5), qh, ())),
                "wl_shm" if state.shm.is_none() => state.shm = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = event else { return };
        if caps.contains(Capability::Pointer) {
            state.pointer.get_or_insert_with(|| seat.get_pointer(qh, ()));
        } else if let Some(p) = state.pointer.take() {
            p.release();
        }
        if caps.contains(Capability::Touch) {
            state.touch.get_or_insert_with(|| seat.get_touch(qh, ()));
        } else if let Some(t) = state.touch.take() {
            t.release();
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(state: &mut Self, _: &WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_pointer::Event::Button { serial, state: WEnum::Value(wl_pointer::ButtonState::Pressed), .. } = event {
            trace!("pointer press, serial {serial}");
            state.shared.lock().unwrap().press_serial = Some(serial);
        }
    }
}

impl Dispatch<WlTouch, ()> for State {
    fn event(state: &mut Self, _: &WlTouch, event: wl_touch::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_touch::Event::Down { serial, id, x, y, .. } => {
                trace!("touch down #{id} at ({x:.0}, {y:.0}), serial {serial}");
                state.shared.lock().unwrap().press_serial = Some(serial);
            }
            wl_touch::Event::Up { id, .. } => trace!("touch up #{id}"),
            wl_touch::Event::Cancel => trace!("touch cancel"),
            _ => {}
        }
    }
}

/// Objects for the drag icon, whose events need nothing done.
macro_rules! ignore_events {
    ($($t:ty),*) => {$(
        impl Dispatch<$t, ()> for State {
            fn event(_: &mut Self, _: &$t, _: <$t as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}
ignore_events!(WlCompositor, WlShm, WlShmPool, WlSurface, WlBuffer);

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &WlDataDeviceManager, _: <WlDataDeviceManager as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(state: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_device::Event::Enter { serial, x, y, id, .. } => {
                let Some(offer) = id else { return };
                let data = offer.data::<OfferData>();
                let files = data.is_some_and(|d| d.uris.load(Ordering::Relaxed));
                let image = data.and_then(OfferData::image_type).map(|(mime, _)| mime);
                trace!("drag entered at ({x:.0}, {y:.0}), files: {files}, image: {image:?}");
                let Some(mime) = (if files { Some(URI_LIST.to_string()) } else { image }) else {
                    offer.accept(serial, None);
                    offer.destroy();
                    return;
                };
                offer.accept(serial, Some(mime));
                if offer.version() >= 3 {
                    // Files dropped on an explorer move there, as they do within the window
                    // (Ctrl copies; see `App::file_drops`). A terminal just types their paths.
                    offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Move);
                }
                if let Some(old) = state.offer.replace(offer) {
                    old.destroy();
                }
                state.track((x, y));
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if state.offer.is_some() {
                    state.track((x, y));
                }
            }
            wl_data_device::Event::Leave => {
                trace!("drag left");
                if let Some(offer) = state.offer.take() {
                    offer.destroy();
                }
                state.end_drag();
            }
            wl_data_device::Event::Drop => {
                let at = state.at;
                trace!("drop at {at:?}, files: {}", state.offer.is_some());
                if let (Some(offer), Some(at)) = (state.offer.take(), at) {
                    let dest = if state.shared.lock().unwrap().on_window(at) {
                        Some(Dest::Here(at))
                    } else {
                        state.relay_target(at).map(|(pid, at)| Dest::Relay(pid, at))
                    };
                    trace!("dropping to {}", match &dest { Some(Dest::Here(_)) => "this window".to_string(), Some(Dest::Relay(pid, _)) => format!("nex {pid}"), None => "nowhere".to_string() });
                    match dest {
                        Some(dest) => state.receive_drop(offer, dest),
                        // Off the window and not over another nex: nothing takes it.
                        None => offer.destroy(),
                    }
                }
                // The drop message ends the hover over a relayed-to window.
                state.relaying_to = None;
                state.end_drag();
            }
            wl_data_device::Event::Selection { id } => {
                trace!("clipboard offered: {}", id.is_some());
                if let Some(old) = std::mem::replace(&mut state.shared.lock().unwrap().selection, id) {
                    old.destroy();
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferData::default()),
    ]);
}

impl Dispatch<WlDataOffer, OfferData> for State {
    fn event(_: &mut Self, _: &WlDataOffer, event: wl_data_offer::Event, data: &OfferData, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if mime_type == URI_LIST {
                    data.uris.store(true, Ordering::Relaxed);
                }
                data.types.lock().unwrap().push(mime_type);
            }
            wl_data_offer::Event::Action { dnd_action: WEnum::Value(action) } => {
                trace!("offer: compositor chose {action:?}");
                data.action.store(action.bits(), Ordering::Relaxed);
            }
            wl_data_offer::Event::SourceActions { source_actions } => trace!("offer: source allows {source_actions:?}"),
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, SourceData> for State {
    fn event(state: &mut Self, source: &WlDataSource, event: wl_data_source::Event, data: &SourceData, _: &Connection, _: &QueueHandle<Self>) {
        trace!("source: {event:?}");
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                let body = if mime_type == URI_LIST { &data.uri_list } else { &data.text };
                let _ = std::fs::File::from(fd).write_all(body.as_bytes());
            }
            wl_data_source::Event::Cancelled | wl_data_source::Event::DndFinished => {
                if let Some(icon) = data.icon.lock().unwrap().take() {
                    icon.destroy();
                }
                source.destroy();
                let _ = state.conn.flush();
                // If the receiver moved the files, they're gone from the listings here.
                crate::file_explorer::files_changed();
                state.ctx.request_repaint();
            }
            _ => {}
        }
    }
}

/// `path` as a `file://` URI, percent-encoding everything but unreserved characters and `/`.
fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_round_trip() {
        let dir = std::env::temp_dir().join(format!("nex uri ü%#{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a b&c.txt");
        std::fs::write(&file, "x").unwrap();
        let uri = file_uri(&file);
        assert!(!uri.contains(' '));
        assert_eq!(parse_paths(&format!("{uri}\r\n")), vec![file]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dropped_images_prefer_png() {
        let offer = |types: &[&str]| OfferData { types: Mutex::new(types.iter().map(|t| t.to_string()).collect()), ..Default::default() };
        let pick = |types: &[&str]| offer(types).image_type();
        // Firefox: the page's URL as a uri-list, the image in several formats.
        assert_eq!(pick(&["text/x-moz-url", URI_LIST, "text/html", "image/jpeg", PNG]), Some((PNG.into(), "png".into())));
        assert_eq!(pick(&["image/svg+xml"]), Some(("image/svg+xml".into(), "svg".into())));
        assert_eq!(pick(&[URI_LIST, TEXT]), None);
    }
}
