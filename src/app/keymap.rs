//! Keyboard shortcuts and clipboard keys, per [`ShortcutStyle`]: Ghostty's Linux defaults
//! (Ctrl+Shift chords) or its macOS ones (Cmd chords). Both tables are always compiled, so
//! the tests check each of them on any OS; `platform::SHORTCUTS` picks the one in use.

use std::ops::BitOr;

use egui::{Event, Key, Modifiers};

use crate::app::layout::{Axis, Direction};
use crate::platform::{self, ShortcutStyle};

use super::actions::Action;

/// The exact modifiers a shortcut needs. `cmd` is the macOS Command key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) struct Mods {
    ctrl: bool,
    shift: bool,
    alt: bool,
    cmd: bool,
}

const NONE: Mods = Mods { ctrl: false, shift: false, alt: false, cmd: false };
const CTRL: Mods = Mods { ctrl: true, ..NONE };
const SHIFT: Mods = Mods { shift: true, ..NONE };
const ALT: Mods = Mods { alt: true, ..NONE };
const CMD: Mods = Mods { cmd: true, ..NONE };

impl BitOr for Mods {
    type Output = Mods;
    fn bitor(self, o: Mods) -> Mods {
        Mods { ctrl: self.ctrl | o.ctrl, shift: self.shift | o.shift, alt: self.alt | o.alt, cmd: self.cmd | o.cmd }
    }
}

impl Mods {
    /// Exactly these modifiers are held (no more, no fewer).
    fn held(self, m: Modifiers) -> bool {
        self == Mods { ctrl: m.ctrl, shift: m.shift, alt: m.alt, cmd: m.mac_cmd }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Binding {
    mods: Mods,
    key: Key,
    action: Action,
}

/// The shortcut for `key` pressed with `mods`, if there is one.
pub(super) fn lookup(bindings: &[Binding], key: Key, mods: Modifiers) -> Option<Action> {
    bindings.iter().find(|b| b.key == key && b.mods.held(mods)).map(|b| b.action)
}

/// The first shortcut for `action`, written out for a tooltip ("Ctrl+Shift+W", "Cmd+D").
pub(super) fn shortcut_label(bindings: &[Binding], action: Action) -> Option<String> {
    let b = bindings.iter().find(|b| b.action == action)?;
    let style = platform::SHORTCUTS;
    let mut parts = Vec::new();
    if b.mods.cmd {
        parts.push("Cmd");
    }
    if b.mods.ctrl {
        parts.push("Ctrl");
    }
    if b.mods.alt {
        parts.push(style.alt_key_name());
    }
    if b.mods.shift {
        parts.push("Shift");
    }
    parts.push(b.key.name());
    Some(parts.join("+"))
}

/// The shortcuts for this OS.
pub(super) fn keybindings() -> Vec<Binding> {
    bindings_for(platform::SHORTCUTS)
}

fn bindings_for(style: ShortcutStyle) -> Vec<Binding> {
    let table = match style {
        ShortcutStyle::Ctrl => ctrl_style(),
        ShortcutStyle::Cmd => cmd_style(),
    };
    table.into_iter().map(|(mods, key, action)| Binding { mods, key, action }).collect()
}

const DIGITS: [Key; 9] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];

/// Linux and Windows.
fn ctrl_style() -> Vec<(Mods, Key, Action)> {
    use Action::*;
    let mut b = vec![
        (CTRL | SHIFT, Key::T, NewTab),
        (CTRL | SHIFT, Key::N, NewWindow),
        (CTRL | SHIFT, Key::W, ClosePane(0)),
        (CTRL | SHIFT, Key::Q, Quit),
        (CTRL | SHIFT, Key::O, Split(Axis::Horizontal)),
        (CTRL | SHIFT, Key::E, Split(Axis::Vertical)),
        (CTRL | SHIFT, Key::Enter, ToggleZoom),
        (CTRL | ALT, Key::ArrowLeft, Focus(Direction::Left)),
        (CTRL | ALT, Key::ArrowRight, Focus(Direction::Right)),
        (CTRL | ALT, Key::ArrowUp, Focus(Direction::Up)),
        (CTRL | ALT, Key::ArrowDown, Focus(Direction::Down)),
        (CTRL | SHIFT | ALT, Key::ArrowLeft, ResizeSplit(Direction::Left)),
        (CTRL | SHIFT | ALT, Key::ArrowRight, ResizeSplit(Direction::Right)),
        (CTRL | SHIFT | ALT, Key::ArrowUp, ResizeSplit(Direction::Up)),
        (CTRL | SHIFT | ALT, Key::ArrowDown, ResizeSplit(Direction::Down)),
        (CTRL | SHIFT | ALT, Key::Equals, Equalize),
        (CTRL | SHIFT | ALT, Key::Plus, Equalize),
        (CTRL, Key::Tab, NextTab),
        (CTRL | SHIFT, Key::Tab, PrevTab),
        (CTRL, Key::PageDown, NextTab),
        (CTRL, Key::PageUp, PrevTab),
        (CTRL | SHIFT, Key::ArrowRight, NextTab),
        (CTRL | SHIFT, Key::ArrowLeft, PrevTab),
        (CTRL, Key::Equals, FontBigger),
        (CTRL, Key::Plus, FontBigger),
        (CTRL | SHIFT, Key::Plus, FontBigger),
        (CTRL | SHIFT, Key::Equals, FontBigger),
        (CTRL, Key::Minus, FontSmaller),
        (CTRL, Key::Num0, FontReset),
        (CTRL | SHIFT, Key::Comma, ReloadConfig),
        (CTRL | SHIFT, Key::K, ClearScreen),
        (CTRL | SHIFT, Key::F, ToggleSearch),
        (CTRL, Key::E, ExploreHere),
        (SHIFT, Key::PageUp, ScrollPageUp),
        (SHIFT, Key::PageDown, ScrollPageDown),
        (SHIFT, Key::Home, ScrollTop),
        (SHIFT, Key::End, ScrollBottom),
    ];
    // Like Ghostty, Alt+9 is "last tab".
    b.extend(DIGITS.into_iter().enumerate().map(|(i, k)| (ALT, k, GotoTab(i))));
    b
}

/// macOS. Plain Ctrl and Option chords are left to the shell.
fn cmd_style() -> Vec<(Mods, Key, Action)> {
    use Action::*;
    let mut b = vec![
        (CMD, Key::T, NewTab),
        (CMD, Key::N, NewWindow),
        (CMD, Key::W, ClosePane(0)),
        (CMD, Key::Q, Quit),
        (CMD, Key::D, Split(Axis::Horizontal)),
        (CMD | SHIFT, Key::D, Split(Axis::Vertical)),
        (CMD | SHIFT, Key::Enter, ToggleZoom),
        (CMD | ALT, Key::ArrowLeft, Focus(Direction::Left)),
        (CMD | ALT, Key::ArrowRight, Focus(Direction::Right)),
        (CMD | ALT, Key::ArrowUp, Focus(Direction::Up)),
        (CMD | ALT, Key::ArrowDown, Focus(Direction::Down)),
        (CMD | CTRL, Key::ArrowLeft, ResizeSplit(Direction::Left)),
        (CMD | CTRL, Key::ArrowRight, ResizeSplit(Direction::Right)),
        (CMD | CTRL, Key::ArrowUp, ResizeSplit(Direction::Up)),
        (CMD | CTRL, Key::ArrowDown, ResizeSplit(Direction::Down)),
        (CMD | CTRL, Key::Equals, Equalize),
        // Shift turns `]` into `}` on most layouts, so both spellings are bound.
        (CMD | SHIFT, Key::CloseBracket, NextTab),
        (CMD | SHIFT, Key::CloseCurlyBracket, NextTab),
        (CMD | SHIFT, Key::OpenBracket, PrevTab),
        (CMD | SHIFT, Key::OpenCurlyBracket, PrevTab),
        (CTRL, Key::Tab, NextTab),
        (CTRL | SHIFT, Key::Tab, PrevTab),
        (CMD, Key::Equals, FontBigger),
        (CMD, Key::Plus, FontBigger),
        (CMD | SHIFT, Key::Plus, FontBigger),
        (CMD | SHIFT, Key::Equals, FontBigger),
        (CMD, Key::Minus, FontSmaller),
        (CMD, Key::Num0, FontReset),
        (CMD | SHIFT, Key::Comma, ReloadConfig),
        (CMD, Key::K, ClearScreen),
        (CMD, Key::F, ToggleSearch),
        (CMD, Key::E, ExploreHere),
        (SHIFT, Key::PageUp, ScrollPageUp),
        (SHIFT, Key::PageDown, ScrollPageDown),
        (SHIFT, Key::Home, ScrollTop),
        (SHIFT, Key::End, ScrollBottom),
        (CMD, Key::ArrowUp, ScrollTop),
        (CMD, Key::ArrowDown, ScrollBottom),
    ];
    b.extend(DIGITS.into_iter().enumerate().map(|(i, k)| (CMD, k, GotoTab(i))));
    b
}

/// What a copy / cut / paste event (egui makes them from the OS's clipboard keys) means in a
/// terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClipboardKey {
    /// Use the system clipboard.
    Clipboard,
    /// Send this control character to the program instead (Ctrl+C → SIGINT).
    Control(u8),
    Ignore,
}

pub(super) fn clipboard_key(event: &Event, mods: Modifiers) -> ClipboardKey {
    clipboard_key_for(platform::SHORTCUTS, event, mods)
}

fn clipboard_key_for(style: ShortcutStyle, event: &Event, mods: Modifiers) -> ClipboardKey {
    use ClipboardKey::*;
    match style {
        // egui turns every Ctrl+C / X / V into a clipboard event, but in a terminal only the
        // Ctrl+Shift ones are clipboard keys; plain Ctrl sends the control character.
        ShortcutStyle::Ctrl => match event {
            Event::Copy if mods.shift || !mods.ctrl => Clipboard,
            Event::Copy => Control(0x03),
            Event::Cut => Control(0x18),
            Event::Paste(_) if mods.shift => Clipboard,
            Event::Paste(_) => Control(0x16),
            _ => Ignore,
        },
        // Only Cmd makes clipboard events here; Ctrl+C arrives as a key.
        ShortcutStyle::Cmd => match event {
            Event::Copy | Event::Paste(_) => Clipboard,
            _ => Ignore,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_chord_is_bound_twice() {
        for style in [ShortcutStyle::Ctrl, ShortcutStyle::Cmd] {
            let mut seen = HashSet::new();
            for b in bindings_for(style) {
                assert!(seen.insert((b.mods, b.key)), "{style:?}: {:?}+{:?} bound twice", b.mods, b.key);
            }
        }
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let ctrl_shift = Modifiers { ctrl: true, shift: true, command: true, ..Default::default() };
        let b = bindings_for(ShortcutStyle::Ctrl);
        assert_eq!(lookup(&b, Key::T, ctrl_shift), Some(Action::NewTab));
        assert_eq!(lookup(&b, Key::T, Modifiers { shift: false, ..ctrl_shift }), None);

        let cmd = Modifiers { mac_cmd: true, command: true, ..Default::default() };
        let b = bindings_for(ShortcutStyle::Cmd);
        assert_eq!(lookup(&b, Key::T, cmd), Some(Action::NewTab));
        assert_eq!(lookup(&b, Key::T, Modifiers { ctrl: true, ..Default::default() }), None);
    }

    #[test]
    fn labels_spell_out_the_chord() {
        let b = bindings_for(ShortcutStyle::Ctrl);
        assert_eq!(shortcut_label(&b, Action::ClosePane(0)).as_deref(), Some("Ctrl+Shift+W"));
        assert_eq!(shortcut_label(&b, Action::CloseExplorer), None);
    }

    #[test]
    fn ctrl_c_is_sigint_only_in_ctrl_style() {
        let ctrl = Modifiers { ctrl: true, command: true, ..Default::default() };
        assert_eq!(clipboard_key_for(ShortcutStyle::Ctrl, &Event::Copy, ctrl), ClipboardKey::Control(0x03));
        let ctrl_shift = Modifiers { shift: true, ..ctrl };
        assert_eq!(clipboard_key_for(ShortcutStyle::Ctrl, &Event::Copy, ctrl_shift), ClipboardKey::Clipboard);
        let cmd = Modifiers { mac_cmd: true, command: true, ..Default::default() };
        assert_eq!(clipboard_key_for(ShortcutStyle::Cmd, &Event::Copy, cmd), ClipboardKey::Clipboard);
        assert_eq!(clipboard_key_for(ShortcutStyle::Cmd, &Event::Paste("x".into()), cmd), ClipboardKey::Clipboard);
    }
}
