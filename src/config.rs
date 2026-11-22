//! Ghostty-style configuration: a plain `key = value` file in the platform's config folder
//! (`$XDG_CONFIG_HOME/nex/config`, falling back to `~/.config/nex/config`, on Linux and
//! macOS), or the file given with `--config`.
//!
//! Unknown keys are reported on stderr and ignored, so a typo never prevents startup.
//! The file is reloaded when it changes, or on Ctrl+Shift+, (see `app/reload.rs`).

use egui::Color32;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

/// The smallest and largest font size, in points: for the config, zooming a pane and a
/// saved session alike.
pub const FONT_MIN: f32 = 6.0;
pub const FONT_MAX: f32 = 72.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Bar,
    Underline,
}

/// Where the tabs are (`tab-bar`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabBar {
    /// In the title bar across the top.
    Top,
    /// A left sidebar of large icon tiles, with touch-sized pane-header buttons and dialogs:
    /// for phone-sized touch screens.
    Sidebar,
}

/// How switching tabs animates (`tab-animation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAnimation {
    /// The new tab soaks through the old one from a drop under its tab.
    Ink,
    /// The old tab glides out the way the tab pill is moving as the new one glides in behind it.
    Glide,
    /// Switch instantly.
    None,
}

/// When a pane's ssh host gets this machine's microphone (`ssh-mic`, see `remote::mic`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshMic {
    /// Never: the "Share microphone" button and shortcut are hidden.
    Off,
    /// When asked, with the pane header's mic button or the shortcut.
    Manual,
    /// As soon as a pane is ssh'd into a host nex can log in to without a password.
    Auto,
}

/// Whether nex puts `nex-host`'s shim on ssh hosts (`ssh-agent-shim`), so `claude` and
/// `codex` typed in its ssh panes keep running after the pane closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentShim {
    /// Never.
    Off,
    /// Offer it once per host (an alert); Not Now isn't asked again.
    Ask,
    /// On every host, without asking.
    On,
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
    /// The dot on a tab whose program is waiting for input; unset follows `palette` 3 (yellow).
    pub tab_attention_color: Option<Color32>,

    pub background_opacity: f32,
    pub unfocused_split_opacity: f32,
    pub window_padding: f32,
    pub window_decorations: bool,
    /// Show the close / minimize / maximize buttons in the title bar.
    pub window_controls: bool,
    pub pane_headers: bool,
    /// Animate pane splits / closes, focus changes and the title-bar tabs.
    pub animations: bool,
    /// How switching tabs animates (when `animations` is on).
    pub tab_animation: TabAnimation,
    /// How new file explorers lay out their folder (`explorer-view = list | grid`).
    pub explorer_view: crate::file_explorer::View,
    /// Where the tabs are (`tab-bar = top | sidebar`; the old `mobile = true` means sidebar).
    pub tab_bar: TabBar,

    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub copy_on_select: bool,
    /// Show a "Copied …" / "Pasted …" toast for terminal copy and paste.
    pub clipboard_toasts: bool,
    /// Files dropped or pasted into a pane running `ssh` are copied to the remote host,
    /// and their remote paths typed (see `remote`).
    pub ssh_upload: bool,
    /// Splitting a pane running `ssh`, or opening a tab from it, opens another session on the
    /// same host in the same remote directory (see `remote::SshTarget::session_command`).
    pub ssh_follow: bool,
    /// Agents on ssh hosts in the Agents pane: nex installs `nex-host` on hosts its panes
    /// connect to and watches the agents it keeps running there (see `remote::host`).
    pub ssh_agents: bool,
    /// `nex-host`'s shim on hosts (see [`AgentShim`]).
    pub ssh_agent_shim: AgentShim,
    /// Sharing this machine's microphone with a pane's ssh host (see `remote::mic`).
    pub ssh_mic: SshMic,
    pub scrollback_limit: usize,
    /// Command to run instead of the user's login shell.
    pub command: Option<String>,
    /// Bring back the last session's tabs, splits and folders at launch (`restore-session`).
    pub restore_session: bool,
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
            tab_attention_color: None,
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
            window_decorations: crate::platform::WINDOW.decorations,
            window_controls: crate::platform::WINDOW.controls,
            pane_headers: true,
            animations: true,
            tab_animation: TabAnimation::Glide,
            explorer_view: crate::file_explorer::View::List,
            tab_bar: if crate::platform::WINDOW.sidebar_tabs { TabBar::Sidebar } else { TabBar::Top },

            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            copy_on_select: false,
            clipboard_toasts: true,
            ssh_upload: true,
            ssh_follow: true,
            ssh_agents: true,
            ssh_agent_shim: AgentShim::Ask,
            ssh_mic: SshMic::Manual,
            scrollback_limit: 10_000,
            command: None,
            restore_session: true,
        }
    }
}

/// A config file given with `--config`, used instead of the default location.
static PATH_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Read the config from `path` from now on (for `--config`). Call before the first `load`.
pub fn set_path(path: PathBuf) {
    let _ = PATH_OVERRIDE.set(path);
}

pub fn config_path() -> PathBuf {
    if let Some(p) = PATH_OVERRIDE.get() {
        return p.clone();
    }
    crate::platform::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("config")
}

/// When the config file was last changed (`None` if it doesn't exist).
pub fn modified() -> Option<SystemTime> {
    std::fs::metadata(config_path()).and_then(|m| m.modified()).ok()
}

impl Config {
    /// Tabs in the left sidebar (`tab-bar = sidebar`), with touch-sized controls.
    pub fn sidebar(&self) -> bool {
        self.tab_bar == TabBar::Sidebar
    }

    /// The tab attention dot's color (`tab-attention-color`, else palette 3).
    pub fn attention_color(&self) -> Color32 {
        self.tab_attention_color.unwrap_or(self.palette[3])
    }

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
            "font-size" => self.font_size = float(value)?.clamp(FONT_MIN, FONT_MAX),
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
            "tab-attention-color" => self.tab_attention_color = Some(color(value)?),
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
            "window-controls" => self.window_controls = boolean(value)?,
            "pane-headers" => self.pane_headers = boolean(value)?,
            "animations" => self.animations = boolean(value)?,
            "tab-animation" => {
                self.tab_animation = match value {
                    "ink" => TabAnimation::Ink,
                    "glide" => TabAnimation::Glide,
                    "none" => TabAnimation::None,
                    _ => return Err(format!("invalid tab-animation `{value}`")),
                }
            }
            "explorer-view" => {
                self.explorer_view = match value {
                    "list" => crate::file_explorer::View::List,
                    "grid" => crate::file_explorer::View::Grid,
                    _ => return Err(format!("invalid explorer-view `{value}`")),
                }
            }
            "tab-bar" => {
                self.tab_bar = match value {
                    "top" => TabBar::Top,
                    "sidebar" => TabBar::Sidebar,
                    _ => return Err(format!("invalid tab-bar `{value}`")),
                }
            }
            // The old name of `tab-bar`, still read so existing configs keep working.
            "mobile" => self.tab_bar = if boolean(value)? { TabBar::Sidebar } else { TabBar::Top },
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
            "clipboard-toasts" => self.clipboard_toasts = boolean(value)?,
            "ssh-upload" => self.ssh_upload = boolean(value)?,
            "ssh-follow" => self.ssh_follow = boolean(value)?,
            "ssh-agents" => self.ssh_agents = boolean(value)?,
            "ssh-agent-shim" => {
                self.ssh_agent_shim = match value {
                    "off" | "false" => AgentShim::Off,
                    "ask" => AgentShim::Ask,
                    "on" | "true" => AgentShim::On,
                    _ => return Err(format!("invalid ssh-agent-shim `{value}` (ask, on or off)")),
                }
            }
            "ssh-mic" => {
                self.ssh_mic = match value {
                    "off" | "false" => SshMic::Off,
                    "manual" | "true" => SshMic::Manual,
                    "auto" => SshMic::Auto,
                    _ => return Err(format!("invalid ssh-mic `{value}` (off, manual or auto)")),
                }
            }
            "scrollback-limit" => {
                self.scrollback_limit = value.parse().map_err(|_| "invalid scrollback-limit")?
            }
            "restore-session" => self.restore_session = boolean(value)?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_comments_are_stripped_but_colors_kept() {
        assert_eq!(strip_comment(" block        # block | bar").trim(), "block");
        assert_eq!(strip_comment(" #1f1b1e").trim(), "#1f1b1e");
        assert_eq!(strip_comment(" 4=#6cb6ff  # blue").trim(), "4=#6cb6ff");
    }

    #[test]
    fn tab_attention_color_defaults_to_yellow() {
        let mut cfg = Config::default();
        assert_eq!(cfg.attention_color(), cfg.palette[3]);
        cfg.apply("tab-attention-color", "#ff3b30").unwrap();
        assert_eq!(cfg.attention_color(), Color32::from_rgb(0xff, 0x3b, 0x30));
        assert!(cfg.apply("tab-attention-color", "red-ish").is_err());
    }

    #[test]
    fn tab_bar_is_top_or_sidebar() {
        let mut cfg = Config::default();
        assert_eq!(cfg.tab_bar, TabBar::Top);
        assert!(!cfg.sidebar());
        cfg.apply("tab-bar", "sidebar").unwrap();
        assert!(cfg.sidebar());
        cfg.apply("tab-bar", "top").unwrap();
        assert_eq!(cfg.tab_bar, TabBar::Top);
        assert!(cfg.apply("tab-bar", "left").is_err());
    }

    #[test]
    fn mobile_still_picks_the_tab_bar() {
        let mut cfg = Config::default();
        cfg.apply("mobile", "true").unwrap();
        assert_eq!(cfg.tab_bar, TabBar::Sidebar);
        cfg.apply("mobile", "false").unwrap();
        assert_eq!(cfg.tab_bar, TabBar::Top);
        assert!(cfg.apply("mobile", "yes").is_err());
    }

    #[test]
    fn explorer_view_picks_a_layout() {
        use crate::file_explorer::View;
        let mut cfg = Config::default();
        assert_eq!(cfg.explorer_view, View::List);
        cfg.apply("explorer-view", "grid").unwrap();
        assert_eq!(cfg.explorer_view, View::Grid);
        cfg.apply("explorer-view", "list").unwrap();
        assert_eq!(cfg.explorer_view, View::List);
        assert!(cfg.apply("explorer-view", "icons").is_err());
    }

    #[test]
    fn tab_animation_picks_a_style() {
        let mut cfg = Config::default();
        assert_eq!(cfg.tab_animation, TabAnimation::Glide);
        for (v, want) in [("ink", TabAnimation::Ink), ("none", TabAnimation::None), ("glide", TabAnimation::Glide)] {
            cfg.apply("tab-animation", v).unwrap();
            assert_eq!(cfg.tab_animation, want);
        }
        assert!(cfg.apply("tab-animation", "fade").is_err());
    }

    #[test]
    fn restore_session_defaults_on() {
        let mut cfg = Config::default();
        assert!(cfg.restore_session);
        cfg.apply("restore-session", "false").unwrap();
        assert!(!cfg.restore_session);
        assert!(cfg.apply("restore-session", "no").is_err());
    }
}
