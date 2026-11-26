//! Sharing this machine's microphone with a pane's ssh host (see `remote::mic`): the pane
//! header's mic button and Ctrl/Cmd+Shift+M, what the button shows, and stopping when no pane
//! is connected to that host any more. With `ssh-mic = auto`, panes share without being asked.
//!
//! One share per host (the same `SshTarget`), however many panes are on it: the host has one
//! default mic. Panes whose shell is on the host the window connects to itself (Android) share
//! over nex's own connection to it, named by `hosts::direct_target`.
//!
//! The Android keyboard button's hold-to-talk asks for a share too ([`App::share_mic_for_talk`]).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::layout::PaneId;
use crate::platform;
use crate::remote::audio::CaptureError;
use crate::config::SshMic;
use crate::remote::mic::{Event, MicError, Share};
use crate::remote::{self, RemoteError, SshTarget};
use crate::terminal::Terminal;
use crate::ui::toast::Toast;

use crate::app::App;
use crate::app::ssh::hosts;
use crate::app::ssh::sharing::LoginHelp;

/// How often panes are checked for hosts to share with or stop sharing with.
const CHECK_INTERVAL: Duration = Duration::from_millis(750);

/// The microphone shared with one host.
struct MicShare {
    host: String,
    share: Share,
    /// A program on the host has the mic open: this machine's mic is recording.
    live: bool,
    /// The host is set up (it has said `Waiting` or `Live`): a program there can open the mic.
    ready: bool,
    /// The "can now record" toast was shown.
    announced: bool,
    /// Started from the button or shortcut (not `auto`), so failures are worth an alert.
    asked: bool,
}

/// Every host the microphone is shared with.
#[derive(Default)]
pub(in crate::app) struct Mics {
    shares: HashMap<SshTarget, MicShare>,
    /// Hosts not to share with automatically again (stopped by hand, or failed) while a pane
    /// is still connected to them.
    skip: HashSet<SshTarget>,
    last_check: Option<Instant>,
}

/// What a pane header's mic button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum MicState {
    /// The pane runs ssh; its host doesn't have the mic.
    Off,
    /// Shared, waiting for a program on the host to open it.
    Shared,
    /// Recording: a program on the host has it open.
    Live,
}

impl App {
    /// The host the window is connected to itself, as a share names it.
    fn direct_mic_target(&self) -> Option<SshTarget> {
        self.connect.host.as_deref().map(hosts::direct_target)
    }

    /// The host pane `t` would share the mic with: the window's own host for a pane whose
    /// shell is there, else the host its `ssh` is connected to (as of the last process poll).
    fn mic_target(&self, t: &Terminal) -> Option<SshTarget> {
        if t.is_remote() { self.direct_mic_target() } else { t.last_ssh_target() }
    }

    /// The mic button for pane `id`, if it should have one (it's running `ssh`, or its shell
    /// is on the window's own host).
    pub(in crate::app) fn mic_state(&self, id: PaneId) -> Option<MicState> {
        if self.cfg.ssh_mic == SshMic::Off {
            return None;
        }
        let target = self.mic_target(self.panes.terminal(id)?)?;
        Some(match self.ssh.mics.shares.get(&target) {
            None => MicState::Off,
            Some(s) if s.live => MicState::Live,
            Some(_) => MicState::Shared,
        })
    }

    /// The host pane `id` is ssh'd into, for the mic button's tooltip.
    pub(in crate::app) fn mic_host(&self, id: PaneId) -> Option<String> {
        Some(self.mic_target(self.panes.terminal(id)?)?.host().to_string())
    }

    /// Like [`mic_target`](Self::mic_target), asking the OS about a pane's `ssh` afresh.
    fn mic_target_now(&mut self, id: PaneId) -> Option<SshTarget> {
        let direct = self.direct_mic_target();
        let t = self.panes.terminal_mut(id)?;
        if t.is_remote() { direct } else { t.ssh_target() }
    }

    /// Share the microphone with pane `id`'s ssh host, or stop sharing it.
    pub(in crate::app) fn toggle_mic(&mut self, ctx: &egui::Context, id: PaneId) {
        if self.cfg.ssh_mic == SshMic::Off {
            return;
        }
        let Some(target) = self.mic_target_now(id) else {
            self.chrome.toast = Some(Toast::new(ctx, "The microphone can be shared with a pane running ssh".into()));
            return;
        };
        let host = target.host().to_string();
        if self.ssh.mics.shares.remove(&target).is_some() {
            self.ssh.mics.skip.insert(target);
            self.chrome.toast = Some(Toast::new(ctx, format!("Stopped sharing the microphone with {host}")));
            return;
        }
        self.ssh.mics.skip.remove(&target);
        if self.start_mic(ctx, target, true) {
            self.chrome.toast = Some(Toast::new(ctx, format!("Sharing the microphone with {host}…")));
        }
    }

    /// Hold-to-talk (the Android keyboard button): make sure the focused pane's host has the
    /// mic. Shares even with `ssh-mic = manual`, since holding the button is asking; never with
    /// `off`. False when there's no share to talk through (nothing to share with, `off`, or
    /// the app may not use the mic yet).
    pub(in crate::app) fn share_mic_for_talk(&mut self, ctx: &egui::Context) -> bool {
        if self.cfg.ssh_mic == SshMic::Off {
            return false;
        }
        let Some(target) = self.focused_pane().and_then(|id| self.mic_target_now(id)) else {
            self.chrome.toast = Some(Toast::new(ctx, "Hold to talk works in a pane on an ssh host".into()));
            return false;
        };
        if self.ssh.mics.shares.contains_key(&target) {
            return true;
        }
        self.ssh.mics.skip.remove(&target);
        self.start_mic(ctx, target, true)
    }

    /// Whether the focused pane's host can record from the mic yet: `Some(true)` once its share
    /// has said `Waiting` or `Live`, `None` when it has no share (it failed, or was stopped).
    pub(in crate::app) fn mic_ready_for_talk(&self) -> Option<bool> {
        let target = self.mic_target(self.panes.terminal(self.focused_pane()?)?)?;
        self.ssh.mics.shares.get(&target).map(|s| s.ready)
    }

    /// Start sharing with `target`, over nex's own connection when it's the window's own host.
    /// Not when the app may not use the mic (Android): then the user is asked (if they asked
    /// for the share), and it's false.
    fn start_mic(&mut self, ctx: &egui::Context, target: SshTarget, asked: bool) -> bool {
        if !platform::mic_allowed() {
            // Shared automatically: ask when the user asks for it, not out of the blue.
            if asked {
                platform::ask_mic();
                self.chrome.toast = Some(Toast::long(ctx, "Allow nex to use the microphone, then try again.".into()));
            }
            return false;
        }
        let repaint = ctx.clone();
        let wake = move || repaint.request_repaint();
        let host = target.host().to_string();
        let share = match self.connect.host.clone() {
            Some(direct) if hosts::direct_target(&direct) == target => Share::start_direct(direct, wake),
            _ => Share::start(target.clone(), wake),
        };
        self.ssh.mics.shares.insert(target, MicShare { host, share, live: false, ready: false, announced: false, asked });
        true
    }

    /// Read what the shares report, and start or stop shares as panes come and go.
    pub(in crate::app) fn poll_mics(&mut self, ctx: &egui::Context) {
        let mut failed = Vec::new();
        let mut toasts = Vec::new();
        for (target, s) in &mut self.ssh.mics.shares {
            while let Ok(event) = s.share.events.try_recv() {
                match event {
                    Event::Waiting => {
                        s.live = false;
                        s.ready = true;
                        if !s.announced {
                            s.announced = true;
                            toasts.push(format!("{} can record from this microphone now", s.host));
                        }
                    }
                    Event::Live => {
                        s.live = true;
                        s.ready = true;
                    }
                    Event::Failed(e) => failed.push((target.clone(), e)),
                }
            }
        }
        if let Some(t) = toasts.pop() {
            self.chrome.toast = Some(Toast::new(ctx, t));
        }
        for (target, e) in failed {
            if let Some(s) = self.ssh.mics.shares.remove(&target) {
                self.mic_failed(ctx, &s.host, s.asked, e);
            }
            self.ssh.mics.skip.insert(target);
        }

        if self.ssh.mics.last_check.is_some_and(|t| t.elapsed() < CHECK_INTERVAL) {
            return;
        }
        self.ssh.mics.last_check = Some(Instant::now());
        if self.cfg.ssh_mic == SshMic::Off {
            self.ssh.mics.shares.clear();
            return;
        }
        let present: HashSet<SshTarget> = self.panes.terminals().filter_map(|(_, t)| self.mic_target(t)).collect();
        let direct = self.direct_mic_target();
        // No pane on the host any more: dropping the share stops it.
        self.ssh.mics.shares.retain(|t, _| present.contains(t));
        self.ssh.mics.skip.retain(|t| present.contains(t));
        if self.cfg.ssh_mic == SshMic::Auto {
            for target in present {
                if self.ssh.mics.shares.contains_key(&target) || self.ssh.mics.skip.contains(&target) {
                    continue;
                }
                // With sharing on, wait for the pane's login to finish (its connection is the
                // one nex's rides on), rather than fail while the password is being typed.
                // (nex's own connection to the window's host has no such login.)
                if direct.as_ref() != Some(&target) && remote::sharing_enabled() && !remote::shared_connection_live(&target) {
                    continue;
                }
                self.start_mic(ctx, target, false);
            }
        }
    }

    /// Quitting: stop every share and give the hosts a moment to take nex's block back out
    /// of their `~/.asoundrc`.
    pub(in crate::app) fn stop_mics(&mut self) {
        let shares: Vec<MicShare> = self.ssh.mics.shares.drain().map(|(_, s)| s).collect();
        for s in &shares {
            s.share.halt();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !shares.iter().all(|s| s.share.done()) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn mic_failed(&mut self, ctx: &egui::Context, host: &str, asked: bool, e: MicError) {
        let msg = match e {
            MicError::Capture(CaptureError::PermissionDenied) => {
                format!("nex isn't allowed to use the microphone. {}", crate::platform::MIC_PERMISSION_HINT)
            }
            MicError::Capture(e) => format!("Couldn't open the microphone: {e}"),
            // Shared automatically: these hosts just can't, and saying so for every pane on one
            // would be noise.
            MicError::Remote(RemoteError::NeedsLogin) | MicError::NotLinux | MicError::NoAlsa if !asked => return,
            MicError::Remote(RemoteError::NeedsLogin) => match self.login_help(host, "Share the microphone again") {
                LoginHelp::LogInAgain => {
                    format!("Couldn't share the microphone: log out of {host} (type exit) and ssh back in, then share it again.")
                }
                LoginHelp::Offered => return,
                LoginHelp::None => format!("Couldn't share the microphone: {host} needs a password, and nex can't type one."),
            },
            e => format!("Couldn't share the microphone with {host}: {e}"),
        };
        self.chrome.toast = Some(Toast::long(ctx, msg));
    }
}
