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
//!
//! The screen is one card drawn as a focused terminal pane (the pane's card and header, from
//! `workspace/draw.rs`), with the desktop alert's buttons and recessed panels in it. It is the
//! same egui on every phone: it places the card in `content_rect` (the safe area) and asks
//! `crate::platform` only for the soft keyboard and the clipboard. The card's header shows
//! the nex dog, which wakes up when the screen first shows; once connected, the card and the
//! dog hand over to the panes (`animation/handoff.rs`).

use std::sync::Arc;

use egui::epaint::{ClippedShape, Shadow};
use egui::layers::ShapeIdx;
use egui::{Align, Align2, Color32, FontId, Id, Layout, Painter, Pos2, Rangef, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

use crate::app::header_height;
use crate::app::workspace::draw::{PANE_RADIUS, header_glyph_rect, header_title};
use crate::jobs::Job;
use crate::remote::ssh::Server;
use crate::remote::ssh::device;
use crate::remote::ssh::host::{self, LoginError, RemoteHost};
use crate::ui::alert::{self, BLUE};
use crate::ui::{dog, glide};
use crate::ui::paint::{self, bold};
use crate::ui::theme::{ERROR, HAIRLINE, Theme, white_alpha};

/// The card's width at most, and the room kept either side of it on a narrow screen.
const COLUMN: f32 = 380.0;
const GUTTER: f32 = 20.0;
/// A field row's height, its text size, and how far its label sits from the card's edge.
const ROW: f32 = 52.0;
const FONT: f32 = 16.0;
const INSET: f32 = 16.0;
const LABEL_WIDTH: f32 = 92.0;
/// The card's body under its header: the pane content's inset, 16 at the sides so text lines
/// up with the rows' labels.
const BODY: egui::Margin = egui::Margin { left: 16, right: 16, top: 12, bottom: 16 };
/// The alert's touch-sized button height (times the screen's scale).
const BUTTON: f32 = 42.0;
/// The header dog's side: the glyph's 13pt plus the margin around the dog in its square.
const DOG: f32 = 17.0;
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
    job: Job<Result<Connected, LoginError>>,
    /// Connect on the first frame, without waiting for the button (the form came filled in).
    auto: bool,
    /// The host the panes are on, once connected.
    pub host: Option<Arc<RemoteHost>>,
    /// When the screen first showed: the header dog's intro plays from then.
    shown_at: Option<f64>,
    /// The card as drawn on the last frame (where the handoff starts).
    drawn: Option<DrawnCard>,
}

/// The connect card as a frame drew it: where the handoff to the workspace starts from.
#[derive(Clone)]
pub(in crate::app) struct DrawnCard {
    pub card: Rect,
    /// The header dog.
    pub dog: Rect,
    pub title: &'static str,
    /// What the body (below the header) painted: the handoff fades it out of the card.
    pub body: Vec<ClippedShape>,
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
            job: Job::new(),
            auto: false,
            host: None,
            shown_at: None,
            drawn: None,
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
        let spawned = self.job.start("ssh login", ctx.clone(), move || {
            host::login(&server, password.as_deref(), trust.as_deref(), key_days).map(|host| {
                let shells = host.idle_shells();
                Connected { host, shells }
            })
        });
        if spawned {
            self.step = Step::Connecting;
            self.message = None;
        }
    }

    /// The card as the last frame drew it.
    pub(in crate::app) fn last_card(&self) -> Option<&DrawnCard> {
        self.drawn.as_ref()
    }

    /// Pick up the login's result. Returns the shells to open tabs for when it just worked.
    pub(in crate::app) fn poll(&mut self) -> Option<Vec<String>> {
        let result = self.job.poll()?;
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

    /// Draw the screen over the whole window, and act on what was tapped. `sidebar` is mobile
    /// mode (the sidebar tab bar): the panes' taller headers and touch-sized buttons.
    pub(in crate::app) fn show(&mut self, ctx: &egui::Context, theme: &Theme, key_days: u32, sidebar: bool) {
        if std::mem::take(&mut self.auto) {
            self.start(ctx, key_days);
        }
        let content = ctx.content_rect();
        let now = ctx.input(|i| i.time);
        let since = now - *self.shown_at.get_or_insert(now);
        let dog_t = if glide::animations(ctx) { (since as f32).min(dog::INTRO) } else { dog::INTRO };
        if dog_t < dog::INTRO {
            ctx.request_repaint();
        }
        let scale = if sidebar { 1.15 } else { 1.0 };
        let mut action = None;
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(theme.window)).show(ctx, |ui| {
            // Where the panel reaches under a status bar or notch, the card starts below it.
            let safe_top = (content.top() - ui.max_rect().top()).max(0.0);
            // Scrolls when the keyboard leaves too little room for the whole card.
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                let span = card_span(content);
                let top = ui.max_rect().top() + safe_top + (content.height() * 0.09).clamp(28.0, 96.0);
                let column = Rect::from_min_size(Pos2::new(span.min, top), Vec2::new(span.span(), 0.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(column).layout(Layout::top_down(Align::Min)), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let trust = match &self.step {
                        Step::Trust(fingerprint) => Some(fingerprint.clone()),
                        _ => None,
                    };
                    let title = if trust.is_some() { "Trust This Server?" } else { "Connect to Your Server" };
                    let (a, card, dog, body) = pane_card(ui, theme, sidebar, title, dog_t, |ui| match trust {
                        Some(fingerprint) => self.trust_view(ui, theme, &fingerprint, scale),
                        None => self.form_view(ui, theme, self.step == Step::Connecting, scale),
                    });
                    action = a;
                    self.drawn = Some(DrawnCard { card, dog, title, body });
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

    /// The card's body: a line on what connecting means, the fields, the Connect button and a
    /// note.
    fn form_view(&mut self, ui: &mut Ui, theme: &Theme, busy: bool, scale: f32) -> Option<Action> {
        note(ui, "Every tab is a shell on your server. Nothing runs on this device.", theme.text_bright.gamma_multiply(0.62), Align::Min);
        ui.add_space(8.0);
        let mut fields = vec![
            (Field::Server, "Server", "home.example.com"),
            (Field::User, "User", "required"),
            (Field::Port, "Port", "22"),
        ];
        if self.needs_password {
            fields.push((Field::Password, "Password", "required"));
        }
        let (rows, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW * fields.len() as f32), Sense::hover());
        // On the card's own fill, edge to edge of it.
        let rows = rows.expand2(Vec2::new(BODY.leftf(), 0.0));
        let mut action = None;
        let mut submit = false;
        for (i, &(field, label, hint)) in fields.iter().enumerate() {
            let row = Rect::from_min_size(rows.min + Vec2::new(0.0, ROW * i as f32), Vec2::new(rows.width(), ROW));
            if i > 0 {
                // Up to the card's border, not over it.
                ui.painter().hline(Rangef::new(row.left() + INSET, row.right() - 1.0), row.top(), Stroke::new(1.0_f32, HAIRLINE));
            }
            let (row_action, entered) = self.field_row(ui, theme, row, field, label, hint, busy);
            action = action.or(row_action);
            submit |= entered;
        }
        if let Some((message, error)) = &self.message {
            ui.add_space(8.0);
            note(ui, message, if *error { ERROR } else { theme.text_bright.gamma_multiply(0.62) }, Align::Min);
        }
        ui.add_space(16.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BUTTON * scale), Sense::hover());
        if busy {
            alert::busy_button(ui, rect, "Connecting…", scale);
        } else if alert::button(ui, rect, "Connect", true, scale, theme) {
            submit = true;
        }
        ui.add_space(14.0);
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

    /// The card's body for a host seen for the first time: its fingerprint, to compare before
    /// trusting it, and how to see it on the host.
    fn trust_view(&mut self, ui: &mut Ui, theme: &Theme, fingerprint: &str, scale: f32) -> Option<Action> {
        let host = self.host_field.trim();
        let subtitle = format!("This is the first connection to {host}. Check that this key is your server's before you trust it.");
        note(ui, &subtitle, theme.text_bright.gamma_multiply(0.62), Align::Min);
        ui.add_space(14.0);
        alert::recessed(ui, |ui| {
            ui.label(RichText::new("KEY FINGERPRINT").font(bold(11.0)).color(theme.text_bright.gamma_multiply(0.45)));
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(fingerprint).font(FontId::monospace(14.0)).color(theme.text_bright)).wrap());
        });
        ui.add_space(12.0);
        ui.label(RichText::new("To see it on the server:").size(13.0).color(theme.text_bright.gamma_multiply(0.55)));
        ui.add_space(6.0);
        alert::recessed(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(FINGERPRINT_COMMAND).font(FontId::monospace(12.5)).color(theme.text_bright.gamma_multiply(0.85))).wrap());
        });
        ui.add_space(18.0);
        // As the alert lays out two buttons: the secondary one left, the primary one right.
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BUTTON * scale), Sense::hover());
        let w = (row.width() - 8.0) / 2.0;
        let cancel = Rect::from_min_size(row.min, Vec2::new(w, row.height()));
        let trust = Rect::from_min_size(Pos2::new(row.right() - w, row.top()), Vec2::new(w, row.height()));
        let mut action = None;
        if alert::button(ui, cancel, "Cancel", false, scale, theme) {
            action = Some(Action::Cancel);
        }
        if alert::button(ui, trust, "Trust and Connect", true, scale, theme) {
            action = Some(Action::Trust(fingerprint.to_string()));
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

/// The card's left and right edges: centered in the safe area `content`, a column wide at
/// most, with a gutter either side on a narrow screen.
fn card_span(content: Rect) -> Rangef {
    let width = COLUMN.min(content.width() - 2.0 * GUTTER).max(0.0);
    Rangef::new(content.center().x - width / 2.0, content.center().x + width / 2.0)
}

/// The screen's card, across `ui`'s width: a focused pane's card and header (the nex dog,
/// `dog_t` seconds into its intro, where a pane's glyph goes, and `title`), a hairline under
/// the header, and `body` inside the pane content's inset. The card grows to fit the body,
/// which is laid out first and painted over it. Returns the body's result, the card and the dog.
fn pane_card<R>(ui: &mut Ui, theme: &Theme, sidebar: bool, title: &str, dog_t: f32, body: impl FnOnce(&mut Ui) -> R) -> (R, Rect, Rect, Vec<ClippedShape>) {
    let under = ui.painter().add(Shape::Noop);
    let (header, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), header_height(sidebar)), Sense::hover());
    let font = if sidebar { 15.0 } else { 13.5 };
    let dog = dog_rect(header);
    dog::paint(ui.painter(), dog, dog_t, (theme.card, 0.0));
    header_title(ui.painter(), header, title, theme.text_bright, header.right() - 20.0, font);
    ui.painter().hline(header.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0_f32, HAIRLINE));
    let body_start = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::new().inner_margin(BODY).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui)
    });
    // A copy of what the body painted (galleys and meshes are shared, so it's cheap).
    let body = ui.ctx().graphics(|g| match g.get(ui.layer_id()) {
        Some(list) => list.all_entries().skip(body_start.0 + 1).filter(|s| !matches!(s.shape, Shape::Noop)).cloned().collect(),
        None => Vec::new(),
    });
    let card = Rect::from_min_max(header.min, Pos2::new(header.right(), inner.response.rect.bottom()));
    paint_under(ui, under, |p| paint::card(p, card, PANE_RADIUS, theme.card, 1.0, None));
    (inner.inner, card, dog, body)
}

/// The header dog: centered on the glyph's place, a little larger than the 13pt glyph since
/// its square includes the coat's stroke, and short of the title (which doesn't move).
fn dog_rect(header: Rect) -> Rect {
    let glyph = header_glyph_rect(header);
    Rect::from_center_size(glyph.center(), Vec2::splat(DOG))
}

/// Paint with `paint` where the `Shape::Noop` at `at` was added, under everything painted
/// since: for a card sized by the content drawn on it.
fn paint_under(ui: &Ui, at: ShapeIdx, paint: impl FnOnce(&Painter)) {
    let p = ui.painter();
    let start = p.add(Shape::Noop);
    paint(p);
    let end = p.add(Shape::Noop);
    ui.ctx().graphics_mut(|g| {
        let list = g.entry(p.layer_id());
        let mut shapes = Vec::new();
        for i in start.0 + 1..end.0 {
            list.mutate_shape(ShapeIdx(i), |s| shapes.push(std::mem::replace(&mut s.shape, Shape::Noop)));
        }
        list.set(at, p.clip_rect(), Shape::Vec(shapes));
    });
}

/// Wrapped text across the column, aligned as asked.
fn note(ui: &mut Ui, text: &str, color: Color32, align: Align) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), Layout::top_down(align), |ui| {
        let size = if align == Align::Center { 14.0 } else { 13.5 };
        ui.add(egui::Label::new(RichText::new(text).size(size).color(color)).wrap_mode(egui::TextWrapMode::Wrap).halign(align));
    });
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
    fn the_card_is_centered_in_the_safe_area() {
        let span = |l, r| {
            let s = card_span(Rect::from_min_max(Pos2::new(l, 40.0), Pos2::new(r, 800.0)));
            (s.min, s.max)
        };
        assert_eq!(span(0.0, 1000.0), (310.0, 690.0), "a column wide at most");
        // A phone in landscape, its notch on the left: a gutter either side of the safe area.
        assert_eq!(span(30.0, 390.0), (50.0, 370.0));
        assert_eq!(span(0.0, 30.0), (15.0, 15.0), "never inside out");
    }

    #[test]
    fn a_remote_argument_connects_at_once() {
        let c = Connect::new(true, Some("jr@home.lan:2222"));
        assert!(c.auto);
        assert_eq!((c.host_field.as_str(), c.port.as_str(), c.user.as_str()), ("home.lan", "2222", "jr"));
        assert!(!Connect::new(false, Some("jr@home")).auto, "only where the window works on a host");
    }
}

