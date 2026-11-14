//! Drag and drop for OSes where winit already handles it.
//!
//! On macOS and Windows, winit reports files dragged in from other apps itself, so there is
//! nothing to add. Dragging files *out* to other apps isn't supported there yet.

use std::path::PathBuf;

use egui::{Pos2, RawInput};

/// Never constructed: `new` returns `None`, so the app skips every call.
pub struct Dnd(());

impl Dnd {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Option<Self> {
        None
    }

    /// Hand an explorer drag that left the window to the OS; `false` if it wasn't taken.
    pub fn start_drag(&mut self, _paths: &[PathBuf], _at: Pos2) -> bool {
        false
    }

    /// Raise this window after a drop.
    pub fn focus_window(&self) {}

    /// Add drags reported by the OS to egui's input.
    pub fn feed(&mut self, _ctx: &egui::Context, _raw: &mut RawInput) {}
}
