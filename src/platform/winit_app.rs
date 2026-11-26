//! eframe's winit application, run on nex's own event loop, with a look at events on their
//! way to egui.
//!
//! - **Paste keys:** on Wayland nex reads the clipboard itself (see `Dnd::note_paste_key` in
//!   `linux/dnd/`), but egui swallows a paste key when its own clipboard is empty, so the key
//!   never reaches the app. This notes paste keys before handing every event on to eframe.
//! - **Waking:** when the app comes back (resumed, or focused: some phones only say that
//!   after the screen comes on), nex's own ssh connections are checked, since the phone may
//!   have lost them while it slept (`remote::ssh::woke`).
//! - **Return on iOS:** winit's iOS view hands what the on-screen keyboard types to the app as
//!   characters, Return included (`"\n"`), and egui drops control characters from typed text.
//!   This turns them back into the Enter (and Tab) keys they are; see [`typed_control_as_key`].
//! - **Frame pacing:** on Wayland vsync is off (see `vsync`), so this keeps frames to the
//!   display's refresh rate instead.

use std::time::{Duration, Instant};

use eframe::{EframeWinitApplication, UserEvent};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::WindowId;

use super::Dnd;

pub struct WinitApp<'a> {
    eframe: EframeWinitApplication<'a>,
    mods: ModifiersState,
    /// Frames are paced here (vsync is off).
    pacer: Option<Pacer>,
}

/// Whether to let the driver wait for vsync when presenting a frame.
///
/// Not on Wayland: there a swap with vsync waits for the compositor's frame callback, and
/// compositors send none to a window that can't be seen (on another workspace, say), so the
/// next frame drawn there blocks the main thread until the window is shown again. Hyprland
/// then calls nex "not responding". Wayland compositors never tear, so nothing is lost.
pub fn vsync(event_loop: &EventLoop<UserEvent>) -> bool {
    #[cfg(target_os = "linux")]
    {
        use winit::platform::wayland::EventLoopExtWayland as _;
        !event_loop.is_wayland()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = event_loop;
        true
    }
}

/// Keeps redraws at least one display refresh apart, as vsync would.
struct Pacer {
    interval: Duration,
    last: Option<Instant>,
}

impl Pacer {
    /// Sleep out what's left of the refresh interval since the last frame.
    fn wait(&mut self) {
        if let Some(left) = self.last.and_then(|t| self.interval.checked_sub(t.elapsed())) {
            std::thread::sleep(left);
        }
        self.last = Some(Instant::now());
    }
}

/// One refresh of the fastest monitor (60 Hz if none says).
fn frame_interval(event_loop: &ActiveEventLoop) -> Duration {
    let mhz = event_loop.available_monitors().filter_map(|m| m.refresh_rate_millihertz()).max().unwrap_or(60_000);
    Duration::from_secs_f64(1000.0 / f64::from(mhz.max(1000)))
}

impl<'a> WinitApp<'a> {
    /// `pace`: vsync is off, so keep frames to the display's refresh rate here.
    pub fn new(eframe: EframeWinitApplication<'a>, pace: bool) -> Self {
        let pacer = pace.then(|| Pacer { interval: Duration::from_secs_f64(1.0 / 60.0), last: None });
        Self { eframe, mods: ModifiersState::empty(), pacer }
    }

    /// Run until the window closes, as `eframe::run_native` does.
    #[cfg(not(target_os = "ios"))]
    pub fn run(mut self, mut event_loop: EventLoop<UserEvent>) -> eframe::Result {
        use winit::platform::run_on_demand::EventLoopExtRunOnDemand as _;
        event_loop.run_app_on_demand(&mut self)?;
        Ok(())
    }

    /// Hand the app to UIKit, which runs it until the system ends it: this never returns.
    #[cfg(target_os = "ios")]
    pub fn run(mut self, event_loop: EventLoop<UserEvent>) -> eframe::Result {
        event_loop.run_app(&mut self)?;
        Ok(())
    }
}

/// The keys egui turns into a paste: the Paste key, or Command (Ctrl off macOS) + V. As in
/// egui, V is the key that types "v", or, on layouts without Latin letters, the key where
/// V is on a US keyboard.
fn is_paste(mods: ModifiersState, logical: &Key, physical: PhysicalKey) -> bool {
    if *logical == Key::Named(NamedKey::Paste) {
        return true;
    }
    let command = if cfg!(target_os = "macos") { mods.super_key() } else { mods.control_key() };
    if !command {
        return false;
    }
    match logical {
        Key::Character(c) if c.eq_ignore_ascii_case("v") => true,
        Key::Character(c) if c.is_ascii() => false,
        _ => physical == PhysicalKey::Code(KeyCode::KeyV),
    }
}

/// The named key a typed control character stands for: Return (`"\n"` from the iOS keyboard,
/// `"\r"` from some hardware keyboards) and Tab.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn control_key(text: &str) -> Option<(NamedKey, KeyCode)> {
    match text {
        "\n" | "\r" | "\r\n" => Some((NamedKey::Enter, KeyCode::Enter)),
        "\t" => Some((NamedKey::Tab, KeyCode::Tab)),
        _ => None,
    }
}

/// A key event that typed a control character, as the key it stands for (see [`control_key`]),
/// with no text: egui would drop the text, and the key is what terminals and text fields act on.
#[cfg(target_os = "ios")]
fn typed_control_as_key(event: WindowEvent) -> WindowEvent {
    match event {
        WindowEvent::KeyboardInput { device_id, event: mut key, is_synthetic } => {
            if let Key::Character(c) = &key.logical_key
                && let Some((named, code)) = control_key(c)
            {
                key.logical_key = Key::Named(named);
                key.physical_key = PhysicalKey::Code(code);
                key.text = None;
            }
            WindowEvent::KeyboardInput { device_id, event: key, is_synthetic }
        }
        other => other,
    }
}

impl ApplicationHandler<UserEvent> for WinitApp<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        crate::remote::ssh::woke();
        if let Some(p) = &mut self.pacer {
            p.interval = frame_interval(event_loop);
        }
        self.eframe.resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        #[cfg(target_os = "ios")]
        let event = typed_control_as_key(event);
        match &event {
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::Focused(true) => crate::remote::ssh::woke(),
            WindowEvent::KeyboardInput { event: key, .. }
                if key.state == ElementState::Pressed && is_paste(self.mods, &key.logical_key, key.physical_key) =>
            {
                Dnd::note_paste_key();
                super::paste_key_pressed(self.mods.shift_key());
            }
            WindowEvent::RedrawRequested => {
                if let Some(p) = &mut self.pacer {
                    p.wait();
                }
            }
            _ => {}
        }
        self.eframe.window_event(event_loop, window_id, event);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.eframe.new_events(event_loop, cause);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.eframe.user_event(event_loop, event);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, device_id: DeviceId, event: DeviceEvent) {
        self.eframe.device_event(event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.memory_warning(event_loop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacer_spaces_frames_by_the_interval() {
        let mut p = Pacer { interval: Duration::from_millis(20), last: None };
        let start = Instant::now();
        p.wait();
        assert!(start.elapsed() < Duration::from_millis(10), "the first frame waits for nothing");
        p.wait();
        p.wait();
        assert!(start.elapsed() >= Duration::from_millis(40));
        // A frame long after the last one goes at once.
        std::thread::sleep(Duration::from_millis(25));
        let t = Instant::now();
        p.wait();
        assert!(t.elapsed() < Duration::from_millis(10));
    }

    #[test]
    fn typed_control_characters_are_keys() {
        assert_eq!(control_key("\n"), Some((NamedKey::Enter, KeyCode::Enter)));
        assert_eq!(control_key("\r"), Some((NamedKey::Enter, KeyCode::Enter)));
        assert_eq!(control_key("\t"), Some((NamedKey::Tab, KeyCode::Tab)));
        assert_eq!(control_key("a"), None);
        assert_eq!(control_key("\n\n"), None);
    }

    #[test]
    fn paste_keys() {
        let ctrl = if cfg!(target_os = "macos") { ModifiersState::SUPER } else { ModifiersState::CONTROL };
        let v = PhysicalKey::Code(KeyCode::KeyV);
        let period = PhysicalKey::Code(KeyCode::Period);
        assert!(is_paste(ctrl, &Key::Character("v".into()), v));
        assert!(is_paste(ctrl | ModifiersState::SHIFT, &Key::Character("V".into()), v));
        assert!(!is_paste(ModifiersState::empty(), &Key::Character("v".into()), v));
        // Dvorak: "v" is on the US "." key, and the US V key types "k".
        assert!(is_paste(ctrl, &Key::Character("v".into()), period));
        assert!(!is_paste(ctrl, &Key::Character("k".into()), v));
        // Cyrillic: the V key types "м".
        assert!(is_paste(ctrl, &Key::Character("м".into()), v));
        assert!(is_paste(ModifiersState::empty(), &Key::Named(NamedKey::Paste), PhysicalKey::Code(KeyCode::Paste)));
    }
}
