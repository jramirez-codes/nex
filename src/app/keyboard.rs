//! Keyboard and clipboard input: shortcuts, search typing, and keys sent to the shell.

use egui::{Event, Key, Modifiers};

use crate::file_explorer::FileExplorer;
use crate::input;
use crate::terminal::Terminal;

use super::App;
use super::actions::Action;

impl App {
    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        // The delete confirmation reads Enter / Escape itself; no key may reach a shell or
        // run a shortcut behind it.
        if self.explorers.values().any(|b| b.confirming_delete()) {
            return;
        }
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        // A key press that ran a shortcut also arrives as text (Alt+1 → "1"); drop that text so
        // it doesn't reach the shell too.
        let mut shortcut_ran = false;
        for event in events {
            match event {
                Event::Key { key, pressed: true, modifiers, .. } => shortcut_ran = self.on_key(key, modifiers),
                Event::Text(_) if shortcut_ran => shortcut_ran = false,
                Event::Text(text) => self.on_text(&text, mods),
                Event::Ime(egui::ImeEvent::Commit(text)) => {
                    if let Some(t) = self.focused_terminal() {
                        t.input(text.into_bytes());
                    }
                }
                Event::Copy | Event::Cut | Event::Paste(_) => self.on_clipboard(ctx, event, mods),
                _ => {}
            }
        }
    }

    fn focused_terminal(&mut self) -> Option<&mut Terminal> {
        self.focused_pane().and_then(|id| self.panes.get_mut(&id))
    }

    fn focused_explorer(&mut self) -> Option<&mut FileExplorer> {
        self.focused_pane().and_then(|id| self.explorers.get_mut(&id))
    }

    /// A key press goes to the first taker: a shortcut, a file explorer, the search field, or
    /// the shell. Returns whether it ran a shortcut.
    fn on_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let binding = self.bindings.iter().find(|(c, s, a, k, _)| {
            *k == key && *c == modifiers.ctrl && *s == modifiers.shift && *a == modifiers.alt
        });
        if let Some(&(.., action)) = binding {
            self.pending.push(action);
            return true;
        }
        if let Some(b) = self.focused_explorer() {
            b.key(key, modifiers);
        } else if self.search.is_some() {
            self.search_key(key, modifiers);
        } else if let Some(t) = self.focused_terminal() {
            let mode = *t.term.lock().mode();
            if let Some(bytes) = input::encode_key(key, modifiers, mode) {
                t.input(bytes);
            }
        }
        false
    }

    /// Keys while the search field is open.
    fn search_key(&mut self, key: Key, modifiers: Modifiers) {
        match key {
            Key::Escape => self.search = None,
            Key::Backspace => {
                if let Some(s) = self.search.as_mut() {
                    s.query.pop();
                }
            }
            // Enter = older match (up), Shift+Enter = newer.
            Key::Enter | Key::F3 => self.search_step(!modifiers.shift),
            Key::ArrowUp | Key::ArrowDown => self.search_step(key == Key::ArrowUp),
            _ => {}
        }
    }

    /// Move to the next search match (older if `up`) and scroll it into view.
    fn search_step(&mut self, up: bool) {
        let Some(s) = self.search.as_mut() else { return };
        if let Some(p) = s.step(up) {
            if let Some(t) = self.panes.get(&s.pane) {
                t.term.lock().scroll_to_point(p);
            }
        }
    }

    /// Typed text: into the search field or the shell (Alt prefixes ESC). File explorers and
    /// Ctrl chords don't take text.
    fn on_text(&mut self, text: &str, mods: Modifiers) {
        if mods.ctrl || self.focused_explorer().is_some() {
            return;
        }
        if let Some(s) = self.search.as_mut() {
            s.query.push_str(text);
            return;
        }
        let Some(t) = self.focused_terminal() else { return };
        let mut bytes = Vec::with_capacity(text.len() + 1);
        if mods.alt {
            bytes.push(0x1b);
        }
        bytes.extend_from_slice(text.as_bytes());
        t.input(bytes);
    }

    /// egui-winit turns Ctrl+C / Ctrl+X / Ctrl+V into clipboard events. In a terminal only the
    /// Ctrl+Shift variants are clipboard operations; plain Ctrl sends the control character to
    /// the program (Ctrl+C → SIGINT). In a file explorer these copy / cut / paste files.
    fn on_clipboard(&mut self, ctx: &egui::Context, event: Event, mods: Modifiers) {
        if let Some(b) = self.focused_explorer() {
            match event {
                Event::Paste(text) => b.paste(ctx, None, Some(&text)),
                e => b.copy(ctx, e == Event::Cut),
            }
            return;
        }
        match event {
            Event::Copy if mods.shift || !mods.ctrl => self.pending.push(Action::Copy),
            Event::Copy => self.send_to_focused(&[0x03]),
            Event::Cut => self.send_to_focused(&[0x18]),
            Event::Paste(text) => {
                if let Some(s) = self.search.as_mut() {
                    s.query.push_str(text.lines().next().unwrap_or_default());
                } else if mods.shift {
                    if let Some(t) = self.focused_terminal() {
                        t.paste(&text);
                    }
                } else {
                    self.send_to_focused(&[0x16]);
                }
            }
            _ => {}
        }
    }

    fn send_to_focused(&mut self, bytes: &[u8]) {
        if let Some(t) = self.focused_terminal() {
            t.input(bytes.to_vec());
        }
    }
}
