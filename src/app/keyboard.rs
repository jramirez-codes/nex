//! Keyboard and clipboard input: shortcuts, search typing, and keys sent to the shell.

use egui::{Event, Key, Modifiers};

use crate::file_explorer::FileExplorer;
use crate::platform;
use crate::terminal::input;
use crate::terminal::Terminal;

use super::App;
use super::actions::Action;
use super::keymap::{self, ClipboardKey};

impl App {
    pub(super) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let paste_key = platform::take_paste_key();
        // Explorer dialogs (delete confirmation, Properties) read the keyboard themselves; no
        // key may reach a shell or run a shortcut behind them.
        if self.explorers.values().any(|b| b.modal_open()) || self.upload_modal_open() {
            return;
        }
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let pasted = events.iter().any(|e| matches!(e, Event::Paste(_)));
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
        if let (Some(shift), false) = (paste_key, pasted) {
            self.on_textless_paste(ctx, shift);
        }
    }

    /// A paste key egui made no `Paste` event for: the clipboard holds no text (an image, say).
    /// In an `ssh` session the image or files are uploaded to the remote host; otherwise a
    /// plain Ctrl+V still reaches the program as `^V` (Claude Code reads the image itself).
    fn on_textless_paste(&mut self, ctx: &egui::Context, shift: bool) {
        let Some(id) = self.focused_pane().filter(|id| self.panes.contains_key(id)) else { return };
        if self.search.is_some() {
            return;
        }
        if self.upload_pasted_files(ctx, id) {
            return;
        }
        if !shift && platform::SHORTCUTS == platform::ShortcutStyle::Ctrl {
            self.send_to_focused(&[0x16]);
        }
    }

    /// If pane `id` runs `ssh` and the clipboard holds copied files or an image, upload them
    /// to the remote host (their remote paths are typed once there). Returns whether it did.
    fn upload_pasted_files(&mut self, ctx: &egui::Context, id: crate::app::layout::PaneId) -> bool {
        let Some(target) = self.ssh_target(id) else { return false };
        let files = match &self.dnd {
            Some(dnd) => dnd.clipboard_files(),
            None => platform::clipboard_files(),
        };
        let Some(files) = files else { return false };
        self.upload_clipboard(ctx, id, target, files);
        true
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
        if key == Key::Escape && self.cur_tab().is_some_and(|t| t.drag.is_some()) {
            self.pending.push(Action::CancelPaneDrag);
            return true;
        }
        if let Some(action) = keymap::lookup(&self.bindings, key, modifiers) {
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

    /// Typed text: into a file explorer (its text fields, or type-to-find), the search field or
    /// the shell (Alt prefixes ESC, except on macOS where Option types composed characters).
    /// Ctrl / Cmd chords don't type.
    fn on_text(&mut self, text: &str, mods: Modifiers) {
        if mods.ctrl || mods.mac_cmd {
            return;
        }
        if let Some(b) = self.focused_explorer() {
            b.text(text);
            return;
        }
        if let Some(s) = self.search.as_mut() {
            s.query.push_str(text);
            return;
        }
        let Some(t) = self.focused_terminal() else { return };
        let mut bytes = Vec::with_capacity(text.len() + 1);
        if mods.alt && platform::SHORTCUTS.alt_sends_esc() {
            bytes.push(0x1b);
        }
        bytes.extend_from_slice(text.as_bytes());
        t.input(bytes);
    }

    /// Copy / cut / paste events. In a file explorer these copy / cut / paste files (or text,
    /// in its location or rename field); in a terminal `keymap::clipboard_key` decides between
    /// the clipboard and a control character.
    fn on_clipboard(&mut self, ctx: &egui::Context, event: Event, mods: Modifiers) {
        if let Some(b) = self.focused_explorer().filter(|b| b.editing()) {
            match event {
                Event::Paste(text) => {
                    b.text(text.lines().next().unwrap_or_default());
                }
                e => {
                    if let Some(text) = b.edit_clipboard(e == Event::Cut) {
                        ctx.copy_text(text);
                    }
                }
            }
            return;
        }
        if let Some(b) = self.focused_explorer() {
            match event {
                Event::Paste(text) => b.paste(ctx, None, Some(&text)),
                e => b.copy(ctx, e == Event::Cut),
            }
            return;
        }
        if let (Event::Paste(text), Some(s)) = (&event, self.search.as_mut()) {
            s.query.push_str(text.lines().next().unwrap_or_default());
            return;
        }
        // Ctrl+V or Ctrl+Shift+V in an `ssh` session, with files or an image copied: upload
        // them rather than paste text the remote side can't use.
        if let Event::Paste(_) = event
            && let Some(id) = self.focused_pane().filter(|id| self.panes.contains_key(id))
            && self.upload_pasted_files(ctx, id)
        {
            return;
        }
        match (keymap::clipboard_key(&event, mods), event) {
            (ClipboardKey::Control(byte), _) => self.send_to_focused(&[byte]),
            (ClipboardKey::Clipboard, Event::Copy) => self.pending.push(Action::Copy),
            (ClipboardKey::Clipboard, Event::Paste(text)) => {
                if let Some(t) = self.focused_terminal() {
                    t.paste(&text);
                    self.clipboard_toast(ctx, "Pasted", &text);
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
