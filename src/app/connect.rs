//! The connect screen: what nex shows where panes can't run local shells (Android) and no ssh
//! session is open yet. It asks for the host to connect to; every pane then works there.
//!
//! The form only collects the host so far. Connecting (the in-process ssh client, the
//! one-time password and the device key) is the next step of the Android port.

use egui::{Align, Color32, FontId, Layout, RichText, Vec2};

use crate::ui::theme::{ACCENT, Theme};

const WIDTH: f32 = 340.0;
const FONT: f32 = 16.0;
const ROW: f32 = 44.0;

/// What the user typed, kept while the screen is shown.
pub(in crate::app) struct ConnectForm {
    host: String,
    port: String,
    user: String,
    /// Shown under the button: why the last try didn't connect.
    message: Option<String>,
}

impl Default for ConnectForm {
    fn default() -> Self {
        ConnectForm { host: String::new(), port: "22".into(), user: String::new(), message: None }
    }
}

/// Where to connect, read from the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct ConnectTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl ConnectForm {
    /// The target typed in, or why it isn't one yet.
    fn target(&self) -> Result<ConnectTarget, &'static str> {
        let host = self.host.trim();
        let user = self.user.trim();
        if host.is_empty() {
            return Err("Enter the server's address.");
        }
        if user.is_empty() {
            return Err("Enter your user name on the server.");
        }
        let port = self.port.trim().parse::<u16>().ok().filter(|&p| p != 0).ok_or("The port is a number from 1 to 65535.")?;
        Ok(ConnectTarget { host: host.into(), port, user: user.into() })
    }

    /// Draw the screen over the whole window. Returns the target when Connect is pressed.
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<ConnectTarget> {
        let mut submit = false;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme.window)).show(ctx, |ui| {
            let width = WIDTH.min(ui.available_width() - 32.0);
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.add_space((ui.available_height() * 0.12).max(24.0));
                ui.label(RichText::new("nex").font(crate::ui::paint::bold(34.0)).color(theme.text_bright));
                ui.add_space(6.0);
                ui.label(RichText::new("Connect to your server").size(FONT).color(theme.text_dim));
                ui.add_space(28.0);
                ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(Align::Min), |ui| {
                    submit |= field(ui, theme, "Server", "home.example.com", &mut self.host);
                    submit |= field(ui, theme, "Port", "22", &mut self.port);
                    submit |= field(ui, theme, "User", "you", &mut self.user);
                    ui.add_space(10.0);
                    let button = egui::Button::new(RichText::new("Connect").size(FONT).color(Color32::WHITE))
                        .fill(ACCENT)
                        .corner_radius(ROW / 2.0)
                        .min_size(Vec2::new(width, ROW));
                    submit |= ui.add(button).clicked();
                    if let Some(message) = &self.message {
                        ui.add_space(12.0);
                        ui.label(RichText::new(message).size(FONT - 2.0).color(theme.text_dim));
                    }
                });
            });
        });
        if !submit {
            return None;
        }
        match self.target() {
            Ok(target) => {
                self.message = None;
                Some(target)
            }
            Err(why) => {
                self.message = Some(why.into());
                None
            }
        }
    }

    /// Say why the last try didn't connect.
    pub(in crate::app) fn set_message(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
    }
}

/// A labelled one-line field; `true` when Enter was pressed in it.
fn field(ui: &mut egui::Ui, theme: &Theme, label: &str, hint: &str, value: &mut String) -> bool {
    ui.label(RichText::new(label).size(FONT - 2.0).color(theme.text_dim));
    ui.add_space(4.0);
    let edit = egui::TextEdit::singleline(value)
        .font(FontId::proportional(FONT))
        .text_color(theme.text_bright)
        .hint_text(hint)
        .desired_width(f32::INFINITY)
        .margin(Vec2::new(12.0, 11.0));
    let response = ui.add(edit);
    ui.add_space(12.0);
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(host: &str, port: &str, user: &str) -> ConnectForm {
        ConnectForm { host: host.into(), port: port.into(), user: user.into(), message: None }
    }

    #[test]
    fn reads_a_target() {
        let t = form(" home.lan ", "2222", "jr").target().unwrap();
        assert_eq!(t, ConnectTarget { host: "home.lan".into(), port: 2222, user: "jr".into() });
    }

    #[test]
    fn rejects_missing_or_bad_fields() {
        assert!(form("", "22", "jr").target().is_err());
        assert!(form("home", "22", " ").target().is_err());
        assert!(form("home", "0", "jr").target().is_err());
        assert!(form("home", "ssh", "jr").target().is_err());
    }
}
