//! Connecting to the host every pane works on, where nex is a thin client: the Android app
//! (no local shells) and `nex --remote`. Until a host is connected, this screen fills the
//! window and asks where to connect. Then:
//!
//! 1. With a valid device key for that host, it logs in at once, so a returning user
//!    doesn't see the form at all (it is filled from `remote.json`).
//! 2. A host nex hasn't seen shows its key's fingerprint and waits for the user to trust it.
//! 3. Without a key (first time, or it expired) it asks for the password once, puts a new
//!    key on the host and logs in with that (see `remote/ssh/device.rs`).
//!
//! Logging in runs on a thread; the screen polls it each frame. Once connected, the window
//! opens a tab for each shell this device left open on the host, or a new shell.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use egui::{Align, Color32, FontId, Layout, RichText, Vec2};

use crate::remote::ssh::Server;
use crate::remote::ssh::device;
use crate::remote::ssh::host::{self, LoginError, RemoteHost};
use crate::ui::theme::{ACCENT, Theme};

const WIDTH: f32 = 340.0;
const FONT: f32 = 16.0;
const ROW: f32 = 44.0;

/// A login that worked: the host, and the shells it has that no pane shows.
struct Connected {
    host: RemoteHost,
    shells: Vec<String>,
}

/// Where in connecting the screen is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    /// Filling in the form.
    Form,
    /// Logging in, on a thread.
    Connecting,
    /// A new host: its fingerprint is shown, to trust or not.
    Trust(String),
}

pub(in crate::app) struct Connect {
    /// This window works on a host rather than here (Android, `--remote`).
    pub active: bool,
    host_field: String,
    port: String,
    user: String,
    /// Only asked for when there is no usable key; cleared once the login is over.
    password: String,
    needs_password: bool,
    /// The fingerprint the user trusted for this host, kept for the rest of the login.
    trusted: Option<String>,
    step: Step,
    message: Option<String>,
    job: Option<Receiver<Result<Connected, LoginError>>>,
    /// Connect on the first frame, without waiting for the button (the form came filled in).
    auto: bool,
    /// The host the panes are on, once connected.
    pub host: Option<Arc<RemoteHost>>,
}

impl Connect {
    /// `remote` is `--remote`'s `user@host[:port]`; without it the form is filled from the
    /// host this device last connected to, if any.
    pub(in crate::app) fn new(active: bool, remote: Option<&str>) -> Self {
        let server = remote.and_then(parse_destination).or_else(|| device::load_profile().map(|p| p.server));
        let mut c = Connect {
            active,
            host_field: String::new(),
            port: "22".into(),
            user: String::new(),
            password: String::new(),
            needs_password: false,
            trusted: None,
            step: Step::Form,
            message: None,
            job: None,
            auto: false,
            host: None,
        };
        if active && let Some(s) = server {
            c.host_field = s.host;
            c.port = s.port.to_string();
            c.user = s.user;
            c.auto = true;
        }
        c
    }

    /// The server typed in, or why it isn't one yet.
    fn server(&self) -> Result<Server, &'static str> {
        let host = self.host_field.trim();
        let user = self.user.trim();
        if host.is_empty() {
            return Err("Enter the server's address.");
        }
        if user.is_empty() {
            return Err("Enter your user name on the server.");
        }
        let port = self.port.trim().parse::<u16>().ok().filter(|&p| p != 0).ok_or("The port is a number from 1 to 65535.")?;
        Ok(Server { host: host.into(), port, user: user.into() })
    }

    /// Start logging in, on a thread.
    fn start(&mut self, ctx: &egui::Context, key_days: u32) {
        let server = match self.server() {
            Ok(s) => s,
            Err(why) => {
                self.message = Some(why.into());
                return;
            }
        };
        if self.needs_password && self.password.is_empty() {
            self.message = Some("Enter your password.".into());
            return;
        }
        // Kept until the login is done with it: a new host asks to be trusted first.
        let password = (!self.password.is_empty()).then(|| self.password.clone());
        let trust = self.trusted.clone();
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new().name("ssh login".into()).spawn(move || {
            let result = host::login(&server, password.as_deref(), trust.as_deref(), key_days).map(|host| {
                let shells = host.idle_shells();
                Connected { host, shells }
            });
            let _ = tx.send(result);
            repaint.request_repaint();
        });
        if spawned.is_ok() {
            self.job = Some(rx);
            self.step = Step::Connecting;
            self.message = None;
        }
    }

    /// Pick up the login's result. Returns the shells to open tabs for when it just worked.
    pub(in crate::app) fn poll(&mut self) -> Option<Vec<String>> {
        let result = self.job.as_ref()?.try_recv().ok()?;
        self.job = None;
        self.step = Step::Form;
        match result {
            Ok(Connected { host, shells }) => {
                self.needs_password = false;
                self.password.clear();
                self.message = None;
                self.host = Some(Arc::new(host));
                Some(shells)
            }
            Err(LoginError::UnknownHost { fingerprint }) => {
                self.step = Step::Trust(fingerprint);
                None
            }
            Err(e) => {
                if matches!(e, LoginError::NeedsPassword | LoginError::WrongPassword) {
                    self.needs_password = true;
                }
                if e == LoginError::WrongPassword {
                    self.password.clear();
                }
                self.message = Some(e.to_string());
                None
            }
        }
    }

    /// Draw the screen over the whole window, and act on its buttons.
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context, theme: &Theme, key_days: u32) {
        if std::mem::take(&mut self.auto) {
            self.start(ctx, key_days);
        }
        let mut submit = false;
        let mut trust = None;
        let mut cancel = false;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme.window)).show(ctx, |ui| {
            let width = WIDTH.min(ui.available_width() - 32.0);
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.add_space((ui.available_height() * 0.10).max(24.0));
                ui.label(RichText::new("nex").font(crate::ui::paint::bold(34.0)).color(theme.text_bright));
                ui.add_space(6.0);
                ui.label(RichText::new("Connect to your server").size(FONT).color(theme.text_dim));
                ui.add_space(28.0);
                ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| match self.step.clone() {
                    Step::Connecting => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(format!("Connecting to {}…", self.host_field.trim())).size(FONT).color(theme.text));
                        });
                    }
                    Step::Trust(fingerprint) => {
                        ui.label(RichText::new("First connection to this server").size(FONT).color(theme.text_bright));
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Check that this is your server's key before trusting it (on the server: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub).")
                                .size(FONT - 2.0)
                                .color(theme.text_dim),
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new(&fingerprint).font(FontId::monospace(FONT - 3.0)).color(theme.text_bright));
                        ui.add_space(16.0);
                        if ui.add(button("Trust and connect", ACCENT, width)).clicked() {
                            trust = Some(fingerprint);
                        }
                        ui.add_space(8.0);
                        cancel = ui.add(button("Cancel", Color32::from_white_alpha(18), width)).clicked();
                    }
                    Step::Form => {
                        submit |= field(ui, theme, "Server", "home.example.com", &mut self.host_field, false);
                        submit |= field(ui, theme, "Port", "22", &mut self.port, false);
                        submit |= field(ui, theme, "User", "you", &mut self.user, false);
                        if self.needs_password {
                            submit |= field(ui, theme, "Password", "used once, to set up this device", &mut self.password, true);
                        }
                        ui.add_space(10.0);
                        submit |= ui.add(button("Connect", ACCENT, width)).clicked();
                    }
                });
                if let Some(message) = &self.message {
                    ui.add_space(12.0);
                    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
                        ui.label(RichText::new(message).size(FONT - 2.0).color(theme.text_dim));
                    });
                }
            });
        });
        if let Some(fingerprint) = trust {
            self.trusted = Some(fingerprint);
            self.step = Step::Form;
            self.start(ctx, key_days);
        } else if cancel {
            self.step = Step::Form;
        } else if submit {
            self.start(ctx, key_days);
        }
    }
}

fn button(label: &str, fill: Color32, width: f32) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).size(FONT).color(Color32::WHITE)).fill(fill).corner_radius(ROW / 2.0).min_size(Vec2::new(width, ROW))
}

/// A labelled one-line field; `true` when Enter was pressed in it.
fn field(ui: &mut egui::Ui, theme: &Theme, label: &str, hint: &str, value: &mut String, secret: bool) -> bool {
    ui.label(RichText::new(label).size(FONT - 2.0).color(theme.text_dim));
    ui.add_space(4.0);
    let edit = egui::TextEdit::singleline(value)
        .font(FontId::proportional(FONT))
        .text_color(theme.text_bright)
        .hint_text(hint)
        .password(secret)
        .desired_width(f32::INFINITY)
        .margin(Vec2::new(12.0, 11.0));
    let response = ui.add(edit);
    ui.add_space(12.0);
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// `user@host`, `user@host:port`, or `user@[v6addr]:port`.
fn parse_destination(dest: &str) -> Option<Server> {
    let (user, rest) = dest.trim().split_once('@')?;
    let (host, port) = match rest.strip_prefix('[') {
        Some(v6) => {
            let (host, after) = v6.split_once(']')?;
            (host, after.strip_prefix(':').map_or(Some(22), |p| p.parse().ok())?)
        }
        None => match rest.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().ok()?),
            None => (rest, 22),
        },
    };
    (!user.is_empty() && !host.is_empty() && port != 0).then(|| Server { host: host.into(), port, user: user.into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(host: &str, port: &str, user: &str) -> Connect {
        let mut c = Connect::new(true, Some("x@y"));
        (c.host_field, c.port, c.user) = (host.into(), port.into(), user.into());
        c
    }

    #[test]
    fn reads_a_server() {
        let s = connect(" home.lan ", "2222", "jr").server().unwrap();
        assert_eq!(s, Server { host: "home.lan".into(), port: 2222, user: "jr".into() });
    }

    #[test]
    fn rejects_missing_or_bad_fields() {
        assert!(connect("", "22", "jr").server().is_err());
        assert!(connect("home", "22", " ").server().is_err());
        assert!(connect("home", "0", "jr").server().is_err());
        assert!(connect("home", "ssh", "jr").server().is_err());
    }

    #[test]
    fn destinations() {
        let s = |h: &str, p, u: &str| Some(Server { host: h.into(), port: p, user: u.into() });
        assert_eq!(parse_destination("jr@home.lan"), s("home.lan", 22, "jr"));
        assert_eq!(parse_destination("jr@home.lan:2222"), s("home.lan", 2222, "jr"));
        assert_eq!(parse_destination("jr@[::1]:2222"), s("::1", 2222, "jr"));
        assert_eq!(parse_destination("jr@[::1]"), s("::1", 22, "jr"));
        assert_eq!(parse_destination("home.lan"), None);
        assert_eq!(parse_destination("jr@home:x"), None);
    }

    #[test]
    fn a_remote_argument_connects_at_once() {
        let c = Connect::new(true, Some("jr@home.lan:2222"));
        assert!(c.auto);
        assert_eq!((c.host_field.as_str(), c.port.as_str(), c.user.as_str()), ("home.lan", "2222", "jr"));
        assert!(!Connect::new(false, Some("jr@home")).auto, "only where the window works on a host");
    }
}
