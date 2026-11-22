//! Connection sharing: nex's own ssh connections (uploads, the shared mic, watching agents,
//! following a session into new panes) ride the login typed into a pane, once the user lets
//! nex add its block to `~/.ssh/config` (see `remote::enable_sharing`). This is the dialog
//! that offers it, and the one decision every "host wants a password" error shares.

use egui::Id;

use crate::remote;
use crate::ui::alert::{Alert, AlertChoice, AlertIcon};
use crate::ui::theme::Theme;

use crate::app::App;

/// What the connection-sharing dialog is showing.
#[derive(Clone)]
pub(in crate::app) enum SharingPrompt {
    /// An upload to this host (sharing the microphone with it, or watching its agents) needed
    /// a password: offer to turn sharing on. Then the last step of logging in again ("Drop the file again").
    Offer(String, &'static str),
    /// A split or new tab of an ssh session to this host would log in again: offer to turn
    /// sharing on first (see `follow`).
    OfferSession(String),
    /// Sharing was just turned on: the open session has to be logged into again.
    Reconnect(String, &'static str),
    /// `~/.ssh/config` couldn't be written.
    Failed(String),
}

/// What nex can do when a host wants a password (see [`App::login_help`]).
pub(in crate::app) enum LoginHelp {
    /// Connection sharing is on: logging in to the host again (in its pane) shares the login.
    LogInAgain,
    /// The connection-sharing dialog is now open; it says what to do.
    Offered,
    /// Sharing isn't available (or the user turned it down): nothing to do.
    None,
}

impl App {
    /// `host` wants a password one of nex's own connections (an upload, the mic) can't type:
    /// what nex can do about it. Offering connection sharing shows the dialog, whose button
    /// says `retry` once sharing is on.
    pub(in crate::app) fn login_help(&mut self, host: &str, retry: &'static str) -> LoginHelp {
        if remote::sharing_enabled() {
            // Sharing is on, but this session started before it was (or outside nex).
            LoginHelp::LogInAgain
        } else if remote::SHARING_SUPPORTED && !self.sharing_declined {
            self.sharing_prompt = Some(SharingPrompt::Offer(host.to_string(), retry));
            LoginHelp::Offered
        } else {
            LoginHelp::None
        }
    }

    /// A modal dialog is open over the terminals and takes the keyboard.
    pub(in crate::app) fn ssh_modal_open(&self) -> bool {
        self.sharing_prompt.is_some() || self.agent_prompt.is_some()
    }

    /// Offered when an upload needed a password: Turn On adds the sharing block to
    /// `~/.ssh/config`, then the alert stays up to say the session must be logged into again
    /// (a session already open can't start sharing). Not Now isn't asked again this run.
    pub(in crate::app) fn sharing_dialog(&mut self, ctx: &egui::Context, theme: &Theme) {
        let Some(prompt) = self.sharing_prompt.clone() else { return };
        let (icon, title, body, steps, primary, secondary) = match &prompt {
            SharingPrompt::Offer(host, _) => (
                AlertIcon::Lock,
                format!("{host} needs a password"),
                "nex can share your ssh connection, so uploads, the shared microphone and the \
                 Agents pane reuse the login you already typed. It adds a small nex-only block \
                 to ~/.ssh/config."
                    .to_string(),
                Vec::new(),
                "Turn On",
                Some("Not Now"),
            ),
            SharingPrompt::OfferSession(host) => (
                AlertIcon::Lock,
                format!("{host} needs a password"),
                "nex can share your ssh connection, so new panes on this host reuse one login \
                 instead of asking again. It adds a small nex-only block to ~/.ssh/config."
                    .to_string(),
                Vec::new(),
                "Turn On",
                Some("Not Now"),
            ),
            SharingPrompt::Reconnect(host, again) => (
                AlertIcon::Done,
                "Sharing is on".to_string(),
                "Log in once more to start sharing.".to_string(),
                vec![
                    format!("Type `exit` to log out of {host}"),
                    "ssh back in with your password".to_string(),
                    again.to_string(),
                ],
                "Got It",
                None,
            ),
            SharingPrompt::Failed(e) => (
                AlertIcon::Warning,
                "Couldn't turn on sharing".to_string(),
                format!("~/.ssh/config couldn't be updated: {e}"),
                Vec::new(),
                "OK",
                None,
            ),
        };
        let alert = Alert { id: Id::new("nex-ssh-sharing"), icon, title: &title, body: &body, steps: &steps, primary, secondary, touch: self.cfg.mobile };
        let Some(choice) = alert.show(ctx, theme) else { return };
        self.sharing_prompt = match (prompt, choice) {
            (SharingPrompt::Offer(host, again), AlertChoice::Primary) => Some(match remote::enable_sharing() {
                Ok(()) => SharingPrompt::Reconnect(host, again),
                Err(e) => SharingPrompt::Failed(e.to_string()),
            }),
            // The new pane is the fresh login, so there's nothing to reconnect.
            (SharingPrompt::OfferSession(_), AlertChoice::Primary) => remote::enable_sharing().err().map(|e| SharingPrompt::Failed(e.to_string())),
            (SharingPrompt::Offer(..) | SharingPrompt::OfferSession(_), AlertChoice::Secondary) => {
                self.sharing_declined = true;
                None
            }
            _ => None,
        };
        // A split or tab waiting on the answer opens either way: with sharing on, its login
        // is the one later panes share.
        self.open_pending(ctx);
    }
}
