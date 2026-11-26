//! Files dropped or pasted into a pane running `ssh`: copied to the remote host on a worker
//! thread (see `crate::remote`), then their remote paths typed into the pane. When the host
//! wants a password, nex offers once to turn on connection sharing (see `sharing`), so later
//! uploads reuse the pane's login.

use std::path::PathBuf;
use std::time::Duration;

use crate::jobs::Jobs;
use crate::layout::PaneId;
use crate::platform::ClipboardFiles;
use crate::remote::{self, RemoteError, SshTarget};
use crate::ui::toast::Toast;

use crate::app::App;
use crate::app::ssh::sharing::LoginHelp;

/// Uploads on their way, each with what to do once it's done.
pub(in crate::app) type Uploads = Jobs<Result<Vec<String>, RemoteError>, Upload>;

/// An upload on its way to a pane's remote host.
pub(in crate::app) struct Upload {
    pane: PaneId,
    host: String,
    /// What was sent, typed instead if the upload fails (none for a pasted image, whose
    /// local copy is only a temp file).
    fallback: Vec<PathBuf>,
    /// The temp file a pasted image was saved to, removed once the upload is done.
    temp: Option<PathBuf>,
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
        let up = Upload { pane, host, fallback, temp };
        self.ssh.uploads.push("nex-upload", ctx.clone(), up, move || remote::upload(&target, &paths));
    }

    /// Type the remote paths of finished uploads into their panes.
    pub(in crate::app) fn poll_uploads(&mut self, ctx: &egui::Context) {
        if self.ssh.uploads.is_empty() {
            return;
        }
        for (up, result) in self.ssh.uploads.poll() {
            let result = result.unwrap_or_else(|| Err("the upload stopped".into()));
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
        if !self.ssh.uploads.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn upload_failed(&mut self, ctx: &egui::Context, host: &str, e: RemoteError) {
        let msg = match e {
            RemoteError::Other(e) => format!("Couldn't upload to {host}: {e}"),
            RemoteError::NeedsLogin => match self.login_help(host, "Drop the file again") {
                LoginHelp::LogInAgain => {
                    format!("Couldn't upload: log out of {host} (type exit) and ssh back in, then drop the file again.")
                }
                LoginHelp::Offered => return,
                LoginHelp::None => format!("Couldn't upload: {host} needs a password, and the upload can't type one."),
            },
        };
        self.toast = Some(Toast::long(ctx, msg));
    }

}
