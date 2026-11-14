//! Drag and drop with other Wayland clients: another nex window, a file manager, a browser.
//!
//! winit 0.30 has no drag and drop on Wayland (only on X11), so nex talks to the compositor
//! itself. It shares winit's connection but has its own event queue, dispatched on a
//! background thread:
//!
//! - **Out:** an explorer drag that leaves the window becomes a compositor drag of
//!   `text/uri-list` (`start_drag`). A drag must name the press (or touch) that started it, so
//!   the thread also listens to the seat's pointer and touch for their serials.
//! - **In:** a drag over the window is reported to egui the way winit reports one on X11:
//!   hovered files and the pointer moving with it, then dropped files. The app's existing drop
//!   handling takes it from there (a terminal types the paths, an explorer copies the files).
//! - **Touch:** sway keeps a finger's drag on the window it started in, so this window passes
//!   it on to the nex under the finger itself (see `relay`).

mod relay;

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::{DroppedFile, Event, HoveredFile, PointerButton, Pos2, RawInput, TouchDeviceId, TouchId, TouchPhase};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::{Backend, ObjectId};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::protocol::wl_touch::{self, WlTouch};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

use crate::file_explorer::parse_paths;

/// `NEX_DEBUG_DND=1` logs each step of a drag between windows to stderr.
pub fn log(args: std::fmt::Arguments) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("NEX_DEBUG_DND").is_some()) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        eprintln!("nex dnd {} {}.{:03}: {args}", std::process::id(), t.as_secs() % 100, t.subsec_millis());
    }
}

macro_rules! trace {
    ($($t:tt)*) => { log(format_args!($($t)*)) };
}

const URI_LIST: &str = "text/uri-list";
const TEXT: &str = "text/plain;charset=utf-8";

/// What the background thread has seen, for the UI thread.
#[derive(Default)]
struct Shared {
    /// Serial of the latest button press or touch down on this client.
    press_serial: Option<u32>,
    /// A drag of files is over the window, at this surface-local position.
    hover: Option<(f64, f64)>,
    /// Files dropped on the window, with where.
    dropped: Vec<(Vec<PathBuf>, (f64, f64))>,
    /// The window's size in surface coordinates, once drawn.
    size: Option<(f64, f64)>,
}

impl Shared {
    /// `at` (surface coordinates) is on the window, or its size isn't known yet.
    fn on_window(&self, (x, y): (f64, f64)) -> bool {
        self.size.is_none_or(|(w, h)| x >= 0.0 && y >= 0.0 && x < w && y < h)
    }
}

pub struct WlDnd {
    conn: Connection,
    qh: QueueHandle<State>,
    manager: WlDataDeviceManager,
    device: WlDataDevice,
    surface: WlSurface,
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
}

impl WlDnd {
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

    fn connect(ctx: egui::Context, display: *mut std::ffi::c_void, surface: *mut std::ffi::c_void) -> Result<Self, String> {
        // SAFETY: both pointers come from the live winit window, which outlives the app (and
        // so this struct and its thread, which only use them through libwayland).
        let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
        let surface_id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.map_err(|e| e.to_string())?;
        let surface = WlSurface::from_id(&conn, surface_id).map_err(|e| e.to_string())?;

        let mut queue: EventQueue<State> = conn.new_event_queue();
        let qh = queue.handle();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let _relay = relay::listen(shared.clone(), ctx.clone());
        let mut state = State {
            ctx,
            conn: conn.clone(),
            shared: shared.clone(),
            seat: None,
            manager: None,
            pointer: None,
            touch: None,
            offer: None,
            selection: None,
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

        std::thread::Builder::new()
            .name("nex-wl-dnd".into())
            .spawn(move || {
                while queue.blocking_dispatch(&mut state).is_ok() {}
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { conn, qh, manager, device, surface, shared, hovering: false, _relay, handed_off: None, touch_down: None })
    }

    /// Hand `paths` to the compositor as a drag out of this window, continuing the press that
    /// is dragging them now (at `at`, in egui points). Returns false if there's no press to
    /// continue.
    pub fn start_drag(&mut self, paths: &[PathBuf], at: Pos2) -> bool {
        let Some(serial) = self.shared.lock().unwrap().press_serial else {
            trace!("start_drag: no press serial yet");
            return false;
        };
        trace!("start_drag: {} path(s), serial {serial}", paths.len());
        let data = SourceData {
            uri_list: paths.iter().map(|p| file_uri(p) + "\r\n").collect(),
            text: paths.iter().map(|p| crate::file_explorer::shell_quote(p)).collect::<Vec<_>>().join(" "),
        };
        let source = self.manager.create_data_source(&self.qh, data);
        source.offer(URI_LIST.into());
        source.offer(TEXT.into());
        if source.version() >= 3 {
            source.set_actions(DndAction::Copy);
        }
        self.device.start_drag(Some(&source), &self.surface, None, serial);
        self.handed_off = Some(at);
        self.conn.flush().is_ok()
    }

    /// Files from another window were dropped on this one: bring it to the front (keyboard
    /// focus), as a drop within the window focuses the pane it landed on.
    pub fn focus_window(&self) {
        std::thread::spawn(relay::focus_this_window);
    }

    /// Before each frame: report an incoming drag to egui as winit does on X11.
    pub fn feed(&mut self, ctx: &egui::Context, raw: &mut RawInput) {
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
        for (paths, at) in dropped {
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
}

/// What an offered drag (or selection) carries.
#[derive(Default)]
struct OfferData {
    uris: AtomicBool,
}

/// The background thread's state.
struct State {
    ctx: egui::Context,
    conn: Connection,
    shared: Arc<Mutex<Shared>>,
    seat: Option<WlSeat>,
    manager: Option<WlDataDeviceManager>,
    pointer: Option<WlPointer>,
    touch: Option<WlTouch>,
    /// The drag over the window now, if it carries files.
    offer: Option<WlDataOffer>,
    /// The clipboard's offer to this device, kept only to be destroyed when replaced.
    selection: Option<WlDataOffer>,
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
        let Ok((mut reader, writer)) = std::io::pipe() else { return };
        offer.receive(URI_LIST.into(), writer.as_fd());
        drop(writer);
        let _ = self.conn.flush();
        let (conn, shared, ctx) = (self.conn.clone(), self.shared.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            if offer.version() >= 3 {
                offer.finish();
            }
            offer.destroy();
            let _ = conn.flush();
            let paths = parse_paths(&text);
            trace!("received {} bytes, {} existing path(s)", text.len(), paths.len());
            match dest {
                Dest::Here(at) if !paths.is_empty() => {
                    shared.lock().unwrap().dropped.push((paths, at));
                    ctx.request_repaint();
                }
                Dest::Relay(pid, (x, y)) => relay::send(pid, &format!("drop {x} {y}\n{text}")),
                Dest::Here(_) => {}
            }
        });
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(state: &mut Self, registry: &WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => state.seat = Some(registry.bind(name, version.min(5), qh, ())),
                "wl_data_device_manager" if state.manager.is_none() => state.manager = Some(registry.bind(name, version.min(3), qh, ())),
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

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &WlDataDeviceManager, _: <WlDataDeviceManager as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(state: &mut Self, _: &WlDataDevice, event: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_data_device::Event::Enter { serial, x, y, id, .. } => {
                let Some(offer) = id else { return };
                let files = offer.data::<OfferData>().is_some_and(|d| d.uris.load(Ordering::Relaxed));
                trace!("drag entered at ({x:.0}, {y:.0}), files: {files}");
                if !files {
                    offer.accept(serial, None);
                    offer.destroy();
                    return;
                }
                offer.accept(serial, Some(URI_LIST.into()));
                if offer.version() >= 3 {
                    // Dropped files are always copied (or their paths typed), never moved.
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
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
                if let Some(old) = std::mem::replace(&mut state.selection, id) {
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
        if let wl_data_offer::Event::Offer { mime_type } = event {
            if mime_type == URI_LIST {
                data.uris.store(true, Ordering::Relaxed);
            }
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
                source.destroy();
                let _ = state.conn.flush();
            }
            _ => {}
        }
    }
}

/// `path` as a `file://` URI, percent-encoding everything but unreserved characters and `/`.
pub fn file_uri(path: &std::path::Path) -> String {
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
}
