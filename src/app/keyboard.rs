//! Keyboard and clipboard input: shortcuts, search typing, and keys sent to the shell.


use egui::{
    Event, Key,
};

use crate::input::{self};

use super::App;
use super::actions::Action;

impl App {
    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let focused = self.focused_pane();
        for event in events {
            match event {
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let binding = self.bindings.iter().find(|(c, s, a, k, _)| {
                        *k == key && *c == modifiers.ctrl && *s == modifiers.shift && *a == modifiers.alt
                    });
                    if let Some(&(.., action)) = binding {
                        self.pending.push(action);
                        continue;
                    }
                    if let Some(b) = focused.and_then(|id| self.browsers.get_mut(&id)) {
                        b.key(key, modifiers);
                        continue;
                    }
                    if let Some(s) = self.search.as_mut() {
                        match key {
                            Key::Escape => self.search = None,
                            Key::Backspace => {
                                s.query.pop();
                            }
                            Key::Enter | Key::F3 => {
                                // Enter = older match (up), Shift+Enter = newer.
                                if let Some(p) = s.step(!modifiers.shift) {
                                    if let Some(t) = self.panes.get(&s.pane) {
                                        t.term.lock().scroll_to_point(p);
                                    }
                                }
                            }
                            Key::ArrowUp | Key::ArrowDown => {
                                if let Some(p) = s.step(key == Key::ArrowUp) {
                                    if let Some(t) = self.panes.get(&s.pane) {
                                        t.term.lock().scroll_to_point(p);
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) else { continue };
                    let mode = *t.term.lock().mode();
                    if let Some(bytes) = input::encode_key(key, modifiers, mode) {
                        t.input(bytes);
                    }
                }
                Event::Text(text) => {
                    if mods.ctrl || focused.is_some_and(|id| self.browsers.contains_key(&id)) {
                        continue;
                    }
                    if let Some(s) = self.search.as_mut() {
                        s.query.push_str(&text);
                        continue;
                    }
                    let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) else { continue };
                    let mut bytes = Vec::with_capacity(text.len() + 1);
                    if mods.alt {
                        bytes.push(0x1b);
                    }
                    bytes.extend_from_slice(text.as_bytes());
                    t.input(bytes);
                }
                Event::Ime(egui::ImeEvent::Commit(text)) => {
                    if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                        t.input(text.into_bytes());
                    }
                }
                // egui-winit turns Ctrl+C / Ctrl+X / Ctrl+V into clipboard events. In a terminal
                // only the Ctrl+Shift variants are clipboard operations; plain Ctrl sends the
                // control character to the program (Ctrl+C → SIGINT).
                // In a file browser these copy / cut / paste files.
                Event::Copy | Event::Cut | Event::Paste(_) if focused.is_some_and(|id| self.browsers.contains_key(&id)) => {
                    let Some(b) = focused.and_then(|id| self.browsers.get_mut(&id)) else { continue };
                    match event {
                        Event::Paste(text) => b.paste(ctx, None, Some(&text)),
                        e => b.copy(ctx, e == Event::Cut),
                    }
                }
                Event::Copy => {
                    if mods.shift || !mods.ctrl {
                        self.pending.push(Action::Copy);
                    } else if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                        t.input(vec![0x03]);
                    }
                }
                Event::Cut => {
                    if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                        t.input(vec![0x18]);
                    }
                }
                Event::Paste(text) => {
                    if let Some(s) = self.search.as_mut() {
                        s.query.push_str(text.lines().next().unwrap_or_default());
                        continue;
                    }
                    let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) else { continue };
                    if mods.shift {
                        t.paste(&text);
                    } else {
                        t.input(vec![0x16]);
                    }
                }
                _ => {}
            }
        }
    }
}
