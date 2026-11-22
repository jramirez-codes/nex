//! Files dropped or pasted into a pane running `ssh`: copied to the remote host on a worker
//! thread (see `crate::remote`), then their remote paths typed into the pane. When the host
//! wants a password, nex offers once to turn on connection sharing, so later uploads reuse
//! the pane's login.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use egui::Id;

use crate::layout::PaneId;
use crate::platform::ClipboardFiles;
use crate::remote::{self, SshTarget, UploadError};
use crate::ui::alert::{Alert, AlertChoice, AlertIcon};
use crate::ui::theme::Theme;
use crate::ui::toast::Toast;

use crate::app::App;

/// An upload on its way to a pane's remote host.
pub(in crate::app) struct Upload {
    pane: PaneId,
    host: String,
    /// What was sent, typed instead if the upload fails (none for a pasted image, whose
    /// local copy is only a temp file).
    fallback: Vec<PathBuf>,
    /// The temp file a pasted image was saved to, removed once the upload is done.
    temp: Option<PathBuf>,
    rx: Receiver<Result<Vec<String>, UploadError>>,
}

impl App {
    /// The host pane `id` is connected to with `ssh`, if uploads are on and it is.
    pub(in crate::app) fn ssh_target(&mut self, id: PaneId) -> Option<SshTarget> {
        if !self.cfg.ssh_upload {
            return None;
        }
        self.panes.get_mut(&id)?.ssh_target()
    }

    /// Files dropped on a terminal: their paths typed, or, in an `ssh` session, copied to the
    /// remote host and the remote paths typed.
    pub(in crate::app) fn insert_or_upload(&mut self, ctx: &egui::Context, id: PaneId, paths: Vec<PathBuf>) {
        match self.ssh_target(id) {
            Some(target) => self.start_upload(ctx, id, target, paths.clone(), paths, None),
            None => self.insert_paths(id, &paths),
        }
    }

    /// Files or an image pasted into an `ssh` session: copied to the remote host.
    pub(in crate::app) fn upload_clipboard(&mut self, ctx: &egui::Context, id: PaneId, target: SshTarget, files: ClipboardFiles) {
        match files {
            ClipboardFiles::Paths(paths) => self.start_upload(ctx, id, target, paths.clone(), paths, None),
            ClipboardFiles::Png(png) => match remote::save_png(&png) {
                Ok(path) => self.start_upload(ctx, id, target, vec![path.clone()], Vec::new(), Some(path)),
                Err(e) => self.toast = Some(Toast::long(ctx, format!("Couldn't save the image: {e}"))),
            },
        }
    }

    fn start_upload(&mut self, ctx: &egui::Context, pane: PaneId, target: SshTarget, paths: Vec<PathBuf>, fallback: Vec<PathBuf>, temp: Option<PathBuf>) {
        let host = target.host().to_string();
        let what = match (temp.is_some(), paths.len()) {
            (true, _) => "image".to_string(),
            (false, 1) => "1 file".to_string(),
            (false, n) => format!("{n} files"),
        };
        self.toast = Some(Toast::new(ctx, format!("Uploading {what} to {host}…")));
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::Builder::new()
            .name("nex-upload".into())
            .spawn(move || {
                let _ = tx.send(remote::upload(&target, &paths));
                repaint.request_repaint();
            })
            .ok();
        self.uploads.push(Upload { pane, host, fallback, temp, rx });
    }

    /// Type the remote paths of finished uploads into their panes.
    pub(in crate::app) fn poll_uploads(&mut self, ctx: &egui::Context) {
        if self.uploads.is_empty() {
            return;
        }
        let mut i = 0;
        while i < self.uploads.len() {
            let result = match self.uploads[i].rx.try_recv() {
                Ok(r) => r,
                Err(TryRecvError::Empty) => {
                    i += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => Err("the upload stopped".into()),
            };
            let up = self.uploads.remove(i);
            if let Some(temp) = &up.temp {
                remote::remove_saved(temp);
            }
            match result {
                Ok(remote_paths) => {
                    if let Some(t) = self.panes.get_mut(&up.pane) {
                        t.paste(&format!("{} ", remote_paths.join(" ")));
                    }
                    self.toast = Some(Toast::new(ctx, format!("Uploaded to {}", up.host)));
                }
                Err(e) => {
                    self.upload_failed(ctx, &up.host, e);
                    if !up.fallback.is_empty() {
                        self.insert_paths(up.pane, &up.fallback);
                    }
                }
            }
        }
        if !self.uploads.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn upload_failed(&mut self, ctx: &egui::Context, host: &str, e: UploadError) {
        let msg = match e {
            UploadError::Other(e) => format!("Couldn't upload to {host}: {e}"),
            // Sharing is on, but this session started before it was (or outside nex).
            UploadError::NeedsLogin if remote::sharing_enabled() => {
                format!("Couldn't upload: log out of {host} (type exit) and ssh back in, then drop the file again.")
            }
            UploadError::NeedsLogin if remote::SHARING_SUPPORTED && !self.sharing_declined => {
                self.sharing_prompt = Some(SharingPrompt::Offer(host.to_string(), "Drop the file again"));
                return;
            }
            UploadError::NeedsLogin => format!("Couldn't upload: {host} needs a password, and the upload can't type one."),
        };
        self.toast = Some(Toast::long(ctx, msg));
    }

    /// A modal dialog is open over the terminals and takes the keyboard.
    pub(in crate::app) fn upload_modal_open(&self) -> bool {
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
