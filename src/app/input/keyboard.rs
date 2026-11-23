//! Keyboard and clipboard input: shortcuts, search typing, and keys sent to the shell.

use egui::{Event, Key, Modifiers};

use crate::file_explorer::FileExplorer;
use crate::layout::PaneKind;
use crate::platform;
use crate::terminal::input;
use crate::terminal::Terminal;

use crate::app::App;
use crate::app::actions::Action;
use crate::app::input::keymap::{self, ClipboardKey};

impl App {
    pub(in crate::app) fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let paste_key = platform::take_paste_key();
        // Explorer dialogs (delete confirmation, Properties) read the keyboard themselves; no
        // key may reach a shell or run a shortcut behind them.
        if self.explorers.values().any(|b| b.modal_open()) || self.ssh_modal_open() {
            return;
        }
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let pasted = events.iter().any(|e| matches!(e, Event::Paste(_)));
        // A key press that ran a shortcut also arrives as text (Alt+1 → "1"); drop that text so
        // it doesn't reach the shell too.
        let mut shortcut_ran = false;
        for event in events {
            match event {
                // Ctrl armed by double-tapping the keyboard button: this key is Ctrl+key, as
                // if Ctrl were held (a nex shortcut such as Ctrl+E, else the program gets
                // it), and its text is dropped.
                Event::Key { key, pressed: true, modifiers, .. } if self.take_armed_ctrl() => {
                    self.on_key(key, modifiers | Modifiers::CTRL);
                    shortcut_ran = true;
                }
                Event::Key { key, pressed: true, modifiers, .. } => shortcut_ran = self.on_key(key, modifiers),
                Event::Text(_) if shortcut_ran => shortcut_ran = false,
                Event::Text(text) => {
                    if let Some(rest) = self.armed_ctrl_text(&text) {
                        self.on_text(&rest, mods);
                    }
                }
                Event::Ime(egui::ImeEvent::Commit(text)) => {
                    let Some(text) = self.armed_ctrl_text(&text) else { continue };
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
        // Text the on-screen keyboard committed: a paste from its clipboard history goes in as
        // a paste, a single character as typing.
        for typed in platform::typed_text() {
            match typed {
                platform::Typed::Paste(text) => {
                    if let Some(t) = self.focused_terminal() {
                        t.paste(&text);
                    }
                }
                platform::Typed::Enter => self.key_to_pane(Key::Enter, Modifiers::NONE),
                platform::Typed::Text { delete, text } => {
                    for _ in 0..delete {
                        self.key_to_pane(Key::Backspace, Modifiers::NONE);
                    }
                    let Some(text) = self.armed_ctrl_text(&text) else { continue };
                    self.type_text(&text);
                }
            }
        }
    }

    /// Text the on-screen keyboard typed (outside of key events): as typed text for a
    /// terminal, file explorer or the search field, as keys for an Agents pane (`j`, `k`...).
    fn type_text(&mut self, text: &str) {
        if self.focused_pane().is_some_and(|id| self.pane_kind(id) == Some(PaneKind::Agents)) {
            for c in text.chars() {
                let key = if c == ' ' { Some(Key::Space) } else { Key::from_name(&c.to_ascii_uppercase().to_string()) };
                if let Some(key) = key {
                    self.key_to_pane(key, Modifiers::NONE);
                }
            }
            return;
        }
        self.on_text(text, Modifiers::NONE);
    }

    /// Use up a Ctrl armed with the keyboard button, if one is. Returns whether one was.
    fn take_armed_ctrl(&mut self) -> bool {
        if !std::mem::take(&mut self.keyboard_button.ctrl_armed) {
            return false;
        }
        platform::haptic(platform::Haptic::Release);
        true
    }

    /// Typed `text` with an armed Ctrl applied to its first character, as a held Ctrl would
    /// be: Ctrl+that character runs its nex shortcut if it has one (`e` → Ctrl+E, the file
    /// explorer), else the character goes to the terminal as its control code. The rest is
    /// returned to type as usual (`None` when nothing is left). Without an armed Ctrl, `text`
    /// is returned as it is.
    fn armed_ctrl_text(&mut self, text: &str) -> Option<String> {
        if !self.keyboard_button.ctrl_armed {
            return (!text.is_empty()).then(|| text.to_owned());
        }
        let mut chars = text.chars();
        let first = chars.next()?;
        self.take_armed_ctrl();
        let rest = chars.as_str().to_owned();
        if let Some(action) = Self::ctrl_shortcut(&self.bindings, first) {
            self.pending.push(action);
            return (!rest.is_empty()).then_some(rest);
        }
        let bytes = match input::ctrl_char(first) {
            Some(b) => vec![b],
            None => first.to_string().into_bytes(),
        };
        if let Some(t) = self.focused_terminal() {
            t.input(bytes);
        }
        (!rest.is_empty()).then_some(rest)
    }

    /// The shortcut for Ctrl + the key that types `c` (either case: `e` and `E` are Ctrl+E),
    /// if there is one.
    fn ctrl_shortcut(bindings: &[keymap::Binding], c: char) -> Option<Action> {
        let key = Key::from_name(&c.to_ascii_uppercase().to_string())?;
        keymap::lookup(bindings, key, Modifiers::CTRL)
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
    fn upload_pasted_files(&mut self, ctx: &egui::Context, id: crate::layout::PaneId) -> bool {
        let Some(target) = self.ssh_target(id) else { return false };
        let files = match &self.dnd {
            Some(dnd) => dnd.clipboard_files(),
            None => platform::clipboard_files(),
        };
        let Some(files) = files else { return false };
        self.upload_clipboard(ctx, id, target, files);
        true
    }

    pub(in crate::app) fn focused_terminal(&mut self) -> Option<&mut Terminal> {
        self.focused_pane().and_then(|id| self.panes.get_mut(&id))
    }

    fn focused_explorer(&mut self) -> Option<&mut FileExplorer> {
        self.focused_pane().and_then(|id| self.explorers.get_mut(&id))
    }

    /// A key press goes to the first taker: a shortcut, a file explorer, the search field, or
    /// the shell. Returns whether it ran a shortcut.
    fn on_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if key == Key::Escape && self.pane_drag.is_some() {
            self.pending.push(Action::CancelPaneDrag);
            return true;
        }
        if let Some(action) = keymap::lookup(&self.bindings, key, modifiers) {
            self.pending.push(action);
            return true;
        }
        self.key_to_pane(key, modifiers);
        false
    }

    /// A key for the focused pane (no shortcut lookup): its own keys for an Agents, Changes or
    /// file explorer pane, the search field while it's open, else the shell.
    pub(in crate::app) fn key_to_pane(&mut self, key: Key, modifiers: Modifiers) {
        let focused = self.focused_pane();
        match focused.and_then(|id| self.pane_kind(id).map(|kind| (id, kind))) {
            Some((id, PaneKind::Agents)) => {
                self.agents.get_mut(&id).unwrap().key(key, modifiers);
            }
            Some((id, PaneKind::Diff)) => {
                self.diffs.get_mut(&id).unwrap().key(key, modifiers);
            }
            Some((id, PaneKind::Explorer)) => {
                self.explorers.get_mut(&id).unwrap().key(key, modifiers);
            }
            Some((_, PaneKind::Terminal)) | None if self.search.is_some() => self.search_key(key, modifiers),
            Some((_, PaneKind::Terminal)) | None => {
                if let Some(t) = self.focused_terminal() {
                    let mode = *t.term.lock().mode();
                    if let Some(bytes) = input::encode_key(key, modifiers, mode) {
                        t.input(bytes);
                    }
                }
            }
        }
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
        if let Some(p) = s.step(up)
            && let Some(t) = self.panes.get(&s.pane) {
                t.term.lock().scroll_to_point(p);
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
        // Nothing to copy or paste in an Agents pane.
        if self.focused_pane().is_some_and(|id| self.agents.contains_key(&id)) {
            return;
        }
        // In a Changes pane, Copy copies the selected file's path; nothing pastes there.
        if let Some(d) = self.focused_pane().and_then(|id| self.diffs.get(&id)) {
            if event == Event::Copy
                && let Some(text) = d.copy_text()
            {
                self.clipboard_toast(ctx, "Copied", &text);
                ctx.copy_text(text);
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
