//! Ghostty-style configuration: a plain `key = value` file at
//! `$XDG_CONFIG_HOME/nex/config` (falls back to `~/.config/nex/config`).
//!
//! Unknown keys are reported on stderr and ignored, so a typo never prevents startup.

use egui::Color32;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub font_family: String,
    pub font_size: f32,
    /// Extra line spacing as a fraction of the font's row height (Ghostty: adjust-cell-height).
    pub cell_height_adjust: f32,

    pub background: Color32,
    pub foreground: Color32,
    pub cursor_color: Color32,
    pub selection_background: Color32,
    pub selection_foreground: Option<Color32>,
    pub palette: [Color32; 16],

    pub background_opacity: f32,
    pub unfocused_split_opacity: f32,
    pub window_padding: f32,
    pub window_decorations: bool,
    pub pane_headers: bool,

    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub copy_on_select: bool,
    pub scrollback_limit: usize,
    /// Command to run instead of the user's login shell.
    pub command: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Empty = auto: an installed "… Nerd Font Mono", else the built-in JetBrains Mono NF.
            font_family: String::new(),
            font_size: 14.0,
            cell_height_adjust: 0.12,

            background: hex("1f1b1e"),
            foreground: hex("d9d4dd"),
            cursor_color: hex("e8e3ec"),
            selection_background: hex("4a4048"),
            selection_foreground: None,
            palette: [
                hex("2b2830"), // black
                hex("f2777a"), // red
                hex("3fbf62"), // green
                hex("e8b65a"), // yellow
                hex("6cb6ff"), // blue
                hex("c792ea"), // magenta
                hex("4fc4c9"), // cyan
                hex("c8c2cc"), // white
                hex("6a6574"), // bright black
                hex("ff8f94"), // bright red
                hex("6fdc8a"), // bright green
                hex("ffd27a"), // bright yellow
                hex("94ccff"), // bright blue
                hex("ddb3ff"), // bright magenta
                hex("80e3e7"), // bright cyan
                hex("f5f1f7"), // bright white
            ],

            background_opacity: 0.90,
            unfocused_split_opacity: 0.90,
            window_padding: 10.0,
            window_decorations: false,
            pane_headers: true,

            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            copy_on_select: false,
            scrollback_limit: 10_000,
            command: None,
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("nex").join("config")
}

impl Config {
    pub fn load() -> Self {
        let mut cfg = Config::default();
        let path = config_path();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for (n, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((key, value)) = line.split_once('=') else {
                    eprintln!("nex: {}:{}: expected `key = value`", path.display(), n + 1);
                    continue;
                };
                if let Err(e) = cfg.apply(key.trim(), unquote(strip_comment(value).trim())) {
                    eprintln!("nex: {}:{}: {e}", path.display(), n + 1);
                }
            }
        }
        cfg
    }

    fn apply(&mut self, key: &str, value: &str) -> Result<(), String> {
        let color = |v: &str| parse_color(v).ok_or_else(|| format!("invalid color `{v}`"));
        let float = |v: &str| v.parse::<f32>().map_err(|_| format!("invalid number `{v}`"));
        let boolean = |v: &str| match v {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("invalid boolean `{v}`")),
        };
        match key {
            "font-family" => self.font_family = value.to_string(),
            "font-size" => self.font_size = float(value)?.clamp(6.0, 72.0),
            "adjust-cell-height" => {
                // Accept "10%" or a fraction like "0.1".
                let v = value.trim_end_matches('%');
                let f = float(v)?;
                self.cell_height_adjust = if value.ends_with('%') { f / 100.0 } else { f };
            }
            "background" => self.background = color(value)?,
            "foreground" => self.foreground = color(value)?,
            "cursor-color" => self.cursor_color = color(value)?,
            "selection-background" => self.selection_background = color(value)?,
            "selection-foreground" => self.selection_foreground = Some(color(value)?),
            "palette" => {
                // Ghostty syntax: `palette = 4=#6cb6ff`
                let (idx, col) = value.split_once('=').ok_or("expected `palette = N=#rrggbb`")?;
                let idx: usize = idx.trim().parse().map_err(|_| "invalid palette index")?;
                if idx >= 16 {
                    return Err("only palette entries 0-15 are configurable".into());
                }
                self.palette[idx] = color(col.trim())?;
            }
            "background-opacity" => self.background_opacity = float(value)?.clamp(0.0, 1.0),
            "unfocused-split-opacity" => {
                self.unfocused_split_opacity = float(value)?.clamp(0.15, 1.0)
            }
            "window-padding" | "window-padding-x" => self.window_padding = float(value)?.max(0.0),
            "window-decorations" => self.window_decorations = boolean(value)?,
            "pane-headers" => self.pane_headers = boolean(value)?,
            "cursor-style" => {
                self.cursor_style = match value {
                    "block" => CursorStyle::Block,
                    "bar" => CursorStyle::Bar,
                    "underline" => CursorStyle::Underline,
                    _ => return Err(format!("invalid cursor-style `{value}`")),
                }
            }
            "cursor-style-blink" => self.cursor_blink = boolean(value)?,
            "copy-on-select" => self.copy_on_select = value != "false",
            "scrollback-limit" => {
                self.scrollback_limit = value.parse().map_err(|_| "invalid scrollback-limit")?
            }
            "command" => self.command = Some(value.to_string()).filter(|s| !s.is_empty()),
            _ => return Err(format!("unknown key `{key}`")),
        }
        Ok(())
    }
}

/// Remove a trailing `# comment`. A `#` only starts a comment after whitespace, so color
/// values like `#1f1b1e` are kept.
fn strip_comment(value: &str) -> &str {
    let v = value.trim_start();
    let bytes = v.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'#' && bytes[i - 1].is_ascii_whitespace() {
            return &v[..i];
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_comments_are_stripped_but_colors_kept() {
        assert_eq!(strip_comment(" block        # block | bar").trim(), "block");
        assert_eq!(strip_comment(" #1f1b1e").trim(), "#1f1b1e");
        assert_eq!(strip_comment(" 4=#6cb6ff  # blue").trim(), "4=#6cb6ff");
    }
}

fn unquote(s: &str) -> &str {
    s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s)
}

pub fn parse_color(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some(Color32::from_rgb(p(0)?, p(2)?, p(4)?))
}

fn hex(s: &str) -> Color32 {
    parse_color(s).expect("valid builtin color")
}
