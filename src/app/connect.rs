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

use egui::epaint::Shadow;
use egui::{Align, Align2, Color32, CursorIcon, FontId, Id, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::remote::ssh::Server;
use crate::remote::ssh::device;
use crate::remote::ssh::host::{self, LoginError, RemoteHost};
use crate::ui::alert::{AlertIcon, BLUE, icon_tile};
use crate::ui::paint::bold;
use crate::ui::theme::{ERROR, HAIRLINE, Theme, lighten, white_alpha};

/// The column's width at most, and the room kept either side of it on a narrow screen.
const COLUMN: f32 = 380.0;
const GUTTER: f32 = 20.0;
const ICON: f32 = 68.0;
/// A field row's height, its text size, and how far its label sits from the edge.
const ROW: f32 = 52.0;
const FONT: f32 = 16.0;
const INSET: f32 = 16.0;
const LABEL_WIDTH: f32 = 92.0;
const BUTTON: f32 = 52.0;
const RADIUS: u8 = 14;
/// What shows a host's key fingerprint, run on the host.
const FINGERPRINT_COMMAND: &str = "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub";

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
    /// A line under the fields, and whether it's an error (else it's a hint).
    message: Option<(String, bool)>,
    /// The field whose Paste callout is open (see `field_row`).
    paste_menu: Option<Field>,
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
            paste_menu: None,
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
                self.message = Some((why.into(), true));
                return;
            }
        };
        if self.needs_password && self.password.is_empty() {
            self.message = Some(("Enter your password.".into(), true));
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
                // Being asked for the password isn't an error; everything else is.
                self.message = Some((e.to_string(), e != LoginError::NeedsPassword));
                None
            }
        }
    }

    /// Draw the screen over the whole window, and act on what was tapped.
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context, theme: &Theme, key_days: u32) {
        if std::mem::take(&mut self.auto) {
            self.start(ctx, key_days);
        }
        let mut action = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme.window)).show(ctx, |ui| {
            // Scrolls when the keyboard leaves too little room for the whole column.
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let full = ui.max_rect();
                let width = COLUMN.min(full.width() - 2.0 * GUTTER);
                let top = (ui.ctx().content_rect().height() * 0.09).clamp(28.0, 96.0);
                let column = Rect::from_min_size(Pos2::new(full.center().x - width / 2.0, full.top() + top), Vec2::new(width, 0.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Center)), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    action = match self.step.clone() {
                        Step::Trust(fingerprint) => self.trust_view(ui, theme, &fingerprint),
                        step => self.form_view(ui, theme, step == Step::Connecting),
                    };
                    ui.add_space(32.0);
                });
            });
        });
        match action {
            Some(Action::Connect) => self.start(ctx, key_days),
            Some(Action::Trust(fingerprint)) => {
                self.trusted = Some(fingerprint);
                self.step = Step::Form;
                self.start(ctx, key_days);
            }
            Some(Action::Cancel) => self.step = Step::Form,
            Some(Action::Paste(field)) => {
                self.paste_menu = None;
                if let Some(text) = crate::platform::clipboard_text() {
                    let id = field_id(field);
                    paste_into(ctx, id, self.value(field), &text);
                    ctx.memory_mut(|m| m.request_focus(id));
                }
            }
            None => {}
        }
    }

    fn value(&mut self, field: Field) -> &mut String {
        match field {
            Field::Server => &mut self.host_field,
            Field::User => &mut self.user,
            Field::Port => &mut self.port,
            Field::Password => &mut self.password,
        }
    }

    /// The header, the fields in one rounded group, the Connect button and a note.
    fn form_view(&mut self, ui: &mut Ui, theme: &Theme, busy: bool) -> Option<Action> {
        header(ui, theme, AlertIcon::Lock, "Connect to Your Server", "Every tab is a shell on your server. Nothing runs on this device.");
        ui.add_space(28.0);
        let mut fields = vec![
            (Field::Server, "Server", "home.example.com"),
            (Field::User, "User", "required"),
            (Field::Port, "Port", "22"),
        ];
        if self.needs_password {
            fields.push((Field::Password, "Password", "required"));
        }
        let width = ui.available_width();
        let (group, _) = ui.allocate_exact_size(Vec2::new(width, ROW * fields.len() as f32), Sense::hover());
        let fill = group_fill(theme);
        ui.painter().rect_filled(group, RADIUS, fill);
        ui.painter().rect_stroke(group, RADIUS, Stroke::new(1.0_f32, white_alpha(16)), StrokeKind::Inside);
        let mut action = None;
        let mut submit = false;
        for (i, &(field, label, hint)) in fields.iter().enumerate() {
            let row = Rect::from_min_size(group.min + Vec2::new(0.0, ROW * i as f32), Vec2::new(width, ROW));
            if i > 0 {
                let y = row.top();
                ui.painter().line_segment([Pos2::new(row.left() + INSET, y), Pos2::new(row.right(), y)], Stroke::new(1.0_f32, HAIRLINE));
            }
            let (row_action, entered) = self.field_row(ui, theme, row, field, label, hint, busy);
            action = action.or(row_action);
            submit |= entered;
        }
        if let Some((message, error)) = &self.message {
            ui.add_space(12.0);
            note(ui, message, if *error { ERROR } else { theme.text_bright.gamma_multiply(0.62) }, Align::Min);
        }
        ui.add_space(22.0);
        let label = if busy { "Connecting…" } else { "Connect" };
        if primary_button(ui, label, busy) && !busy {
            submit = true;
        }
        ui.add_space(16.0);
        let footer = if self.needs_password {
            "Your password is used once, to add this device's key to the server. After that, nex signs in with the key."
        } else {
            "nex signs in with this device's own key. The first time, it asks for your password once to set the key up."
        };
        note(ui, footer, theme.text_bright.gamma_multiply(0.42), Align::Center);
        if submit && !busy {
            action = action.or(Some(Action::Connect));
        }
        action
    }

    /// One row of the group: its label, and a borderless field filling the rest. A long
    /// press (or right click) offers Paste above it, where egui has no clipboard of its own.
    #[allow(clippy::too_many_arguments)]
    fn field_row(&mut self, ui: &mut Ui, theme: &Theme, row: Rect, field: Field, label: &str, hint: &str, busy: bool) -> (Option<Action>, bool) {
        let id = field_id(field);
        ui.painter().text(Pos2::new(row.left() + INSET, row.center().y), Align2::LEFT_CENTER, label, FontId::proportional(FONT), theme.text_bright);
        let edit_rect = Rect::from_min_max(Pos2::new(row.left() + INSET + LABEL_WIDTH, row.top()), Pos2::new(row.right() - INSET, row.bottom()));
        let secret = field == Field::Password;
        let edit = egui::TextEdit::singleline(self.value(field))
            .id(id)
            .frame(false)
            .font(FontId::proportional(FONT))
            .text_color(theme.text_bright)
            .hint_text(RichText::new(hint).color(theme.text_bright.gamma_multiply(0.32)))
            .password(secret)
            .interactive(!busy)
            .vertical_align(Align::Center)
            .desired_width(edit_rect.width())
            .margin(Vec2::ZERO);
        let response = ui.scope(|ui| {
            ui.visuals_mut().selection.bg_fill = BLUE.gamma_multiply(0.45);
            ui.visuals_mut().text_cursor.stroke = Stroke::new(2.0_f32, BLUE);
            ui.put(edit_rect, edit)
        }).inner;
        // A tap on the label focuses its field, as in a settings list.
        let label_hit = ui.interact(Rect::from_min_max(row.min, Pos2::new(edit_rect.left(), row.bottom())), id.with("label"), Sense::click());
        if label_hit.clicked() && !busy {
            response.request_focus();
        }
        let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if crate::platform::SOFT_KEYBOARD && !busy && (response.long_touched() || response.secondary_clicked()) {
            self.paste_menu = Some(field);
        }
        let mut action = None;
        if self.paste_menu == Some(field) {
            match paste_pill(ui, row) {
                Pill::Tapped => action = Some(Action::Paste(field)),
                Pill::Dismissed => self.paste_menu = None,
                Pill::Open => {}
            }
        }
        (action, entered)
    }

    /// A host seen for the first time: its fingerprint, to compare before trusting it.
    fn trust_view(&mut self, ui: &mut Ui, theme: &Theme, fingerprint: &str) -> Option<Action> {
        let host = self.host_field.trim().to_string();
        header(
            ui,
            theme,
            AlertIcon::Warning,
            "Trust This Server?",
            &format!("This is the first connection to {host}. Check that this key is your server's before you trust it."),
        );
        ui.add_space(24.0);
        egui::Frame::new().fill(group_fill(theme)).stroke(Stroke::new(1.0_f32, white_alpha(16))).corner_radius(RADIUS).inner_margin(16).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.label(RichText::new("KEY FINGERPRINT").font(bold(11.0)).color(theme.text_bright.gamma_multiply(0.45)));
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(fingerprint).font(FontId::monospace(14.0)).color(theme.text_bright)).wrap());
                ui.add_space(14.0);
                ui.label(RichText::new("To see it on the server:").size(13.0).color(theme.text_bright.gamma_multiply(0.55)));
                ui.add_space(6.0);
                egui::Frame::new().fill(Color32::from_black_alpha(60)).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(FINGERPRINT_COMMAND).font(FontId::monospace(12.5)).color(theme.text_bright.gamma_multiply(0.85))).wrap());
                });
            });
        });
        ui.add_space(24.0);
        let mut action = None;
        if primary_button(ui, "Trust and Connect", false) {
            action = Some(Action::Trust(fingerprint.to_string()));
        }
        ui.add_space(6.0);
        if text_button(ui, "Cancel") {
            action = Some(Action::Cancel);
        }
        action
    }
}

/// What a tap on the screen asks for.
enum Action {
    Connect,
    Trust(String),
    Cancel,
    Paste(Field),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Field {
    Server,
    User,
    Port,
    Password,
}

fn field_id(field: Field) -> Id {
    Id::new(("connect-field", field))
}

/// The cards' fill: the window's card color, a step lighter, opaque.
fn group_fill(theme: &Theme) -> Color32 {
    lighten(theme.card.to_opaque(), 0.06)
}

/// The app badge, a bold title and a line under it, centered.
fn header(ui: &mut Ui, theme: &Theme, icon: AlertIcon, title: &str, subtitle: &str) {
    let (tile, _) = ui.allocate_exact_size(Vec2::splat(ICON), Sense::hover());
    icon_tile(ui.painter(), tile, icon);
    ui.add_space(20.0);
    ui.label(RichText::new(title).font(bold(24.0)).color(theme.text_bright));
    ui.add_space(8.0);
    note(ui, subtitle, theme.text_bright.gamma_multiply(0.62), Align::Center);
}

/// Wrapped text across the column, aligned as asked.
fn note(ui: &mut Ui, text: &str, color: Color32, align: Align) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(align), |ui| {
        let size = if align == Align::Center { 14.0 } else { 13.5 };
        ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap_mode(egui::TextWrapMode::Wrap).halign(align));
    });
}

/// The full-width blue button; with `busy`, a spinner beside its label and no presses.
fn primary_button(ui: &mut Ui, label: &str, busy: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BUTTON), if busy { Sense::hover() } else { Sense::click() });
    let fill = if busy {
        BLUE.gamma_multiply(0.6)
    } else if resp.is_pointer_button_down_on() {
        BLUE.gamma_multiply(0.78)
    } else if resp.hovered() {
        lighten(BLUE, 0.08)
    } else {
        BLUE
    };
    let p = ui.painter();
    p.rect_filled(rect, RADIUS, fill);
    p.rect_stroke(rect, RADIUS, Stroke::new(1.0_f32, white_alpha(30)), StrokeKind::Inside);
    let font = bold(16.0);
    if busy {
        let text_w = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x);
        let spin = 18.0;
        let left = rect.center().x - (text_w + spin + 10.0) / 2.0;
        egui::Spinner::new().size(spin).color(Color32::WHITE).paint_at(ui, Rect::from_center_size(Pos2::new(left + spin / 2.0, rect.center().y), Vec2::splat(spin)));
        p.text(Pos2::new(left + spin + 10.0, rect.center().y), Align2::LEFT_CENTER, label, font, Color32::WHITE);
    } else {
        p.text(rect.center(), Align2::CENTER_CENTER, label, font, Color32::WHITE);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// A plain blue text button, centered (iOS's secondary action).
fn text_button(ui: &mut Ui, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BUTTON), Sense::click());
    let color = if resp.is_pointer_button_down_on() { BLUE.gamma_multiply(0.6) } else { BLUE };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(16.0), color);
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

enum Pill {
    Open,
    Tapped,
    /// A tap somewhere else.
    Dismissed,
}

/// The dark "Paste" callout above a field, as iOS shows on a long press.
fn paste_pill(ui: &mut Ui, row: Rect) -> Pill {
    let size = Vec2::new(84.0, 38.0);
    let at = Pos2::new(row.left() + INSET + LABEL_WIDTH + size.x / 2.0, row.top() - size.y / 2.0 - 4.0);
    let rect = Rect::from_center_size(at, size);
    let area = egui::Area::new(Id::new("connect-paste")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
        let (r, resp) = ui.allocate_exact_size(size, Sense::click());
        let p = ui.painter();
        p.add(Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) }.as_shape(r, 10));
        let fill = if resp.is_pointer_button_down_on() { Color32::from_gray(70) } else { Color32::from_gray(52) };
        p.rect_filled(r, 10, fill);
        p.rect_stroke(r, 10, Stroke::new(1.0_f32, white_alpha(24)), StrokeKind::Inside);
        p.text(r.center(), Align2::CENTER_CENTER, "Paste", FontId::proportional(15.0), Color32::WHITE);
        resp.clicked()
    });
    if area.inner {
        return Pill::Tapped;
    }
    let tapped_elsewhere = ui.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && !ui.input(|i| i.pointer.any_down() && i.pointer.press_start_time().is_some_and(|t| i.time - t > 0.3));
    if tapped_elsewhere { Pill::Dismissed } else { Pill::Open }
}

/// Put `text` in `value` at the field's cursor (over its selection), as one line, and move
/// the cursor past it.
fn paste_into(ctx: &egui::Context, id: Id, value: &mut String, text: &str) {
    let text: String = text.trim().chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let len = value.chars().count();
    let range = state.cursor.char_range().map_or(len..len, |r| r.as_sorted_char_range());
    let byte = |chars: usize| value.char_indices().nth(chars).map_or(value.len(), |(b, _)| b);
    let (start, end) = (byte(range.start.min(len)), byte(range.end.min(len)));
    value.replace_range(start..end, &text);
    let at = range.start.min(len) + text.chars().count();
    state.cursor.set_char_range(Some(egui::text_selection::CCursorRange::one(egui::text::CCursor::new(at))));
    state.store(ctx, id);
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
