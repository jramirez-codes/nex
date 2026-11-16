//! eframe's winit application, run on nex's own event loop, with a look at key presses on
//! their way to egui.
//!
//! On Wayland nex reads the clipboard itself (see `Dnd::note_paste_key` in
//! `linux/dnd/`), but egui swallows a paste key when its own clipboard is empty, so the key
//! never reaches the app. This notes paste keys before handing every event on to eframe.

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
}

impl<'a> WinitApp<'a> {
    pub fn new(eframe: EframeWinitApplication<'a>) -> Self {
        Self { eframe, mods: ModifiersState::empty() }
    }

    /// Run until the window closes, as `eframe::run_native` does.
    pub fn run(mut self, event_loop: &mut EventLoop<UserEvent>) -> eframe::Result {
        use winit::platform::run_on_demand::EventLoopExtRunOnDemand as _;
        event_loop.run_app_on_demand(&mut self)?;
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

impl ApplicationHandler<UserEvent> for WinitApp<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        match &event {
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event: key, .. } if key.state == ElementState::Pressed && is_paste(self.mods, &key.logical_key, key.physical_key) => {
                Dnd::note_paste_key();
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
