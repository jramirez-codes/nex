//! Translate egui keyboard/mouse input into the byte sequences terminal programs expect
//! (xterm conventions).

use alacritty_terminal::term::TermMode;
use egui::{Key, Modifiers};

/// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
fn mod_param(m: Modifiers) -> u8 {
    1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8
}

/// Encode a key press that doesn't produce text on its own (arrows, Enter, Ctrl+letter...).
/// Plain printable characters arrive separately as `Event::Text`.
pub fn encode_key(key: Key, m: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let modified = m.shift || m.alt || m.ctrl;
    let esc_prefix = |mut bytes: Vec<u8>| {
        if m.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };

    // CSI sequences for cursor keys: ESC [ A, ESC O A (application mode), ESC [ 1 ; m A.
    let cursor = |c: u8| -> Vec<u8> {
        if modified {
            format!("\x1b[1;{}{}", mod_param(m), c as char).into_bytes()
        } else if app_cursor {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        }
    };
    // Tilde sequences: ESC [ n ~ or ESC [ n ; m ~.
    let tilde = |n: u8| -> Vec<u8> {
        if modified {
            format!("\x1b[{n};{}~", mod_param(m)).into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    // F1-F4 use SS3: ESC O P, or ESC [ 1 ; m P when modified.
    let ss3 = |c: u8| -> Vec<u8> {
        if modified {
            format!("\x1b[1;{}{}", mod_param(m), c as char).into_bytes()
        } else {
            vec![0x1b, b'O', c]
        }
    };

    let bytes = match key {
        Key::ArrowUp => cursor(b'A'),
        Key::ArrowDown => cursor(b'B'),
        Key::ArrowRight => cursor(b'C'),
        Key::ArrowLeft => cursor(b'D'),
        Key::Home => cursor(b'H'),
        Key::End => cursor(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => ss3(b'P'),
        Key::F2 => ss3(b'Q'),
        Key::F3 => ss3(b'R'),
        Key::F4 => ss3(b'S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),

        Key::Enter => esc_prefix(vec![b'\r']),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => esc_prefix(vec![b'\t']),
        Key::Backspace if m.ctrl => esc_prefix(vec![0x08]),
        Key::Backspace => esc_prefix(vec![0x7f]),
        Key::Escape => esc_prefix(vec![0x1b]),
        Key::Space if m.ctrl => esc_prefix(vec![0]),

        // Ctrl + key → C0 control characters.
        _ if m.ctrl => esc_prefix(vec![ctrl_byte(key)?]),
        _ => return None,
    };
    Some(bytes)
}

fn ctrl_byte(key: Key) -> Option<u8> {
    let name = key.name();
    if name.len() == 1 {
        let c = name.as_bytes()[0];
        if c.is_ascii_alphabetic() {
            return Some(c.to_ascii_uppercase() - b'A' + 1);
        }
    }
    Some(match key {
        Key::OpenBracket => 0x1b,
        Key::Backslash => 0x1c,
        Key::CloseBracket => 0x1d,
        Key::Slash => 0x1f,
        Key::Num2 => 0x00,
        Key::Num3 => 0x1b,
        Key::Num4 => 0x1c,
        Key::Num5 => 0x1d,
        Key::Num6 => 0x1e,
        Key::Num7 => 0x1f,
        Key::Num8 => 0x7f,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    WheelUp = 64,
    WheelDown = 65,
}

pub enum MouseAction {
    Press,
    Release,
    /// Movement; `Some` while a button is held.
    Motion(Option<MouseButton>),
}

/// Encode a mouse report for programs that enabled mouse tracking (vim, htop, lazygit...).
pub fn encode_mouse(
    action: MouseAction,
    button: MouseButton,
    col: usize,
    line: usize,
    m: Modifiers,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let mut code = match action {
        MouseAction::Motion(Some(b)) => b as u8 + 32,
        MouseAction::Motion(None) => 3 + 32,
        _ => button as u8,
    };
    code += 4 * m.shift as u8 + 8 * m.alt as u8 + 16 * m.ctrl as u8;

    if mode.contains(TermMode::SGR_MOUSE) {
        let suffix = if matches!(action, MouseAction::Release) { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{};{}{suffix}", col + 1, line + 1).into_bytes());
    }

    // Legacy X10 encoding: releases are button 3, coordinates offset by 32 and capped at 223.
    if matches!(action, MouseAction::Release) {
        code = 3 + (code & !3);
    }
    if col >= 223 || line >= 223 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + code, 32 + col as u8 + 1, 32 + line as u8 + 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_letters() {
        let ctrl = Modifiers::CTRL;
        assert_eq!(encode_key(Key::C, ctrl, TermMode::empty()), Some(vec![3]));
        assert_eq!(encode_key(Key::A, ctrl, TermMode::empty()), Some(vec![1]));
        assert_eq!(encode_key(Key::Z, ctrl, TermMode::empty()), Some(vec![26]));
    }

    #[test]
    fn arrows_respect_app_cursor_mode() {
        let none = Modifiers::NONE;
        assert_eq!(encode_key(Key::ArrowUp, none, TermMode::empty()).unwrap(), b"\x1b[A");
        assert_eq!(encode_key(Key::ArrowUp, none, TermMode::APP_CURSOR).unwrap(), b"\x1bOA");
        assert_eq!(encode_key(Key::ArrowLeft, Modifiers::CTRL, TermMode::empty()).unwrap(), b"\x1b[1;5D");
    }

    #[test]
    fn sgr_mouse() {
        let bytes = encode_mouse(MouseAction::Release, MouseButton::Left, 4, 9, Modifiers::NONE, TermMode::SGR_MOUSE);
        assert_eq!(bytes.unwrap(), b"\x1b[<0;5;10m");
    }
}
