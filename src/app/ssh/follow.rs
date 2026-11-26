//! Splitting a pane that's running `ssh`, or opening a tab from it, opens another session on
//! the same host, in the same remote directory (`ssh-follow`). With connection sharing on
//! (see `remote::enable_sharing`), the new session reuses the pane's login.

use std::path::PathBuf;

use crate::app::App;
use crate::layout::{Axis, PaneId};
use crate::app::ssh::sharing::SharingPrompt;
use crate::remote;

/// Where a new terminal goes.
#[derive(Clone, Copy, Debug)]
pub(in crate::app) enum Place {
    /// Beside this pane.
    Split(PaneId, Axis),
    Tab,
}

/// A terminal waiting for the user to answer the connection-sharing alert.
#[derive(Clone, Debug)]
pub(in crate::app) struct PendingPane {
    place: Place,
    cwd: Option<PathBuf>,
    command: Vec<String>,
}

impl App {
    /// Open a terminal at `place`, following pane `from`: in its directory, and when it's
    /// running `ssh`, in another session on the same host.
    pub(in crate::app) fn open_from(&mut self, ctx: &egui::Context, from: Option<PaneId>, place: Place) {
        let cwd = from.and_then(|id| self.pane_cwd(id));
        let Some((command, target)) = from.and_then(|id| self.ssh_session(id)) else {
            self.open_terminal(ctx, place, cwd, None);
            return;
        };
        // No shared connection to ride on: offer to turn sharing on first, so the login this
        // pane is about to ask for becomes one later splits can share.
        let offer = remote::SHARING_SUPPORTED
            && !self.ssh.sharing_declined
            && self.ssh.sharing_prompt.is_none()
            && !remote::sharing_enabled()
            && !remote::shared_connection_live(&target);
        if offer {
            self.ssh.pending_pane = Some(PendingPane { place, cwd, command });
            self.ssh.sharing_prompt = Some(SharingPrompt::OfferSession(target.host().to_string()));
        } else {
            self.open_terminal(ctx, place, cwd, Some(command));
        }
    }

    /// Open the terminal that was waiting on the sharing alert, now it's answered.
    pub(in crate::app) fn open_pending(&mut self, ctx: &egui::Context) {
        let Some(p) = self.ssh.pending_pane.take() else { return };
        let place = match p.place {
            Place::Split(target, _) if !self.panes.is_terminal(target) => Place::Tab,
            place => place,
        };
        self.open_terminal(ctx, place, p.cwd, Some(p.command));
    }

    fn open_terminal(&mut self, ctx: &egui::Context, place: Place, cwd: Option<PathBuf>, command: Option<Vec<String>>) {
        match place {
            Place::Split(target, axis) => self.split_at(ctx, target, axis, cwd, command),
            Place::Tab => self.new_tab(ctx, cwd, command),
        }
    }

    /// "Open Terminal Here" in Files pane `pane`: a terminal split off it in `dir`, on the
    /// pane's ssh host when it shows one (a new `ssh` session there, or a new shell on the
    /// host the window is connected to).
    pub(in crate::app) fn open_terminal_in(&mut self, ctx: &egui::Context, pane: PaneId, dir: PathBuf) {
        use crate::remote::exec::Transport;
        let place = Place::Split(pane, Axis::Horizontal);
        let Some(t) = self.panes.explorer(pane).and_then(|b| b.transport().cloned()) else {
            self.open_terminal(ctx, place, Some(dir), None);
            return;
        };
        let dir = dir.to_string_lossy().into_owned();
        match &*t {
            Transport::Ssh(target) => self.open_terminal(ctx, place, None, Some(target.session_command(Some(&dir)))),
            Transport::Nex(_) => {
                self.next_remote_cwd = Some(dir);
                self.open_terminal(ctx, place, None, None);
                self.next_remote_cwd = None;
            }
        }
    }

    /// A Files or Changes pane on `host` couldn't log in there: what nex can do about it.
    pub(in crate::app) fn remote_pane_login_failed(&mut self, ctx: &egui::Context, host: &str) {
        use crate::app::ssh::sharing::LoginHelp;
        let msg = match self.login_help(host, "Done") {
            LoginHelp::LogInAgain => format!("{host} needs a password: log out of it (type exit) and ssh back in, and the pane will catch up."),
            LoginHelp::Offered => return,
            LoginHelp::None => format!("{host} needs a password, and nex's own connection can't type one."),
        };
        self.chrome.toast = Some(crate::ui::toast::Toast::long(ctx, msg));
    }

    /// The command line for another session on the host pane `id` is connected to with
    /// `ssh`, in the remote directory it's in, if following is on and it is.
    fn ssh_session(&mut self, id: PaneId) -> Option<(Vec<String>, remote::SshTarget)> {
        // The command runs under `/bin/sh`.
        if !self.cfg.ssh_follow || !cfg!(unix) {
            return None;
        }
        let target = self.panes.terminal_mut(id)?.ssh_target()?;
        let dir = self.pane_remote_cwd(id);
        Some((target.session_command(dir.as_deref()), target))
    }
}
