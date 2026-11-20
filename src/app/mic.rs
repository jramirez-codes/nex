//! Sharing this machine's microphone with a pane's ssh host (see `remote::mic`): the pane
//! header's mic button and Ctrl/Cmd+Shift+M, what the button shows, and stopping when no pane
//! is connected to that host any more. With `ssh-mic = auto`, panes share without being asked.
//!
//! One share per host (the same `SshTarget`), however many panes are on it: the host has one
//! default mic.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::layout::PaneId;
use crate::remote::audio::CaptureError;
use crate::config::SshMic;
use crate::remote::mic::{Event, MicError, Share};
use crate::remote::{self, SshTarget};
use crate::ui::toast::Toast;

use crate::app::App;
use crate::app::upload::SharingPrompt;

/// How often panes are checked for hosts to share with or stop sharing with.
const CHECK_INTERVAL: Duration = Duration::from_millis(750);

/// The microphone shared with one host.
struct MicShare {
    host: String,
    share: Share,
    /// A program on the host has the mic open: this machine's mic is recording.
    live: bool,
    /// The "can now record" toast was shown.
    announced: bool,
    /// Started from the button or shortcut (not `auto`), so failures are worth an alert.
    asked: bool,
}

/// Every host the microphone is shared with.
#[derive(Default)]
pub(super) struct Mics {
    shares: HashMap<SshTarget, MicShare>,
    /// Hosts not to share with automatically again (stopped by hand, or failed) while a pane
    /// is still connected to them.
    skip: HashSet<SshTarget>,
    last_check: Option<Instant>,
}

/// What a pane header's mic button shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MicState {
    /// The pane runs ssh; its host doesn't have the mic.
    Off,
    /// Shared, waiting for a program on the host to open it.
    Shared,
    /// Recording: a program on the host has it open.
    Live,
}

impl App {
    /// The mic button for pane `id`, if it should have one (it's running `ssh`).
    pub(super) fn mic_state(&self, id: PaneId) -> Option<MicState> {
        if self.cfg.ssh_mic == SshMic::Off {
            return None;
        }
        let target = self.panes.get(&id)?.last_ssh_target()?;
        Some(match self.mics.shares.get(&target) {
            None => MicState::Off,
            Some(s) if s.live => MicState::Live,
            Some(_) => MicState::Shared,
        })
    }

    /// The host pane `id` is ssh'd into, for the mic button's tooltip.
    pub(super) fn mic_host(&self, id: PaneId) -> Option<String> {
        Some(self.panes.get(&id)?.last_ssh_target()?.host().to_string())
    }

    /// Share the microphone with pane `id`'s ssh host, or stop sharing it.
    pub(super) fn toggle_mic(&mut self, ctx: &egui::Context, id: PaneId) {
        if self.cfg.ssh_mic == SshMic::Off {
            return;
        }
        let Some(target) = self.panes.get_mut(&id).and_then(|t| t.ssh_target()) else {
            self.toast = Some(Toast::new(ctx, "The microphone can be shared with a pane running ssh".into()));
            return;
        };
        let host = target.host().to_string();
        if self.mics.shares.remove(&target).is_some() {
            self.mics.skip.insert(target);
            self.toast = Some(Toast::new(ctx, format!("Stopped sharing the microphone with {host}")));
            return;
        }
        self.mics.skip.remove(&target);
        self.toast = Some(Toast::new(ctx, format!("Sharing the microphone with {host}…")));
        self.start_mic(ctx, target, true);
    }

    fn start_mic(&mut self, ctx: &egui::Context, target: SshTarget, asked: bool) {
        let repaint = ctx.clone();
        let host = target.host().to_string();
        let share = Share::start(target.clone(), move || repaint.request_repaint());
        self.mics.shares.insert(target, MicShare { host, share, live: false, announced: false, asked });
    }

    /// Read what the shares report, and start or stop shares as panes come and go.
    pub(super) fn poll_mics(&mut self, ctx: &egui::Context) {
        let mut failed = Vec::new();
        let mut toasts = Vec::new();
        for (target, s) in &mut self.mics.shares {
            while let Ok(event) = s.share.events.try_recv() {
                match event {
                    Event::Waiting => {
                        s.live = false;
                        if !s.announced {
                            s.announced = true;
                            toasts.push(format!("{} can record from this microphone now", s.host));
                        }
                    }
                    Event::Live => s.live = true,
                    Event::Failed(e) => failed.push((target.clone(), e)),
                }
            }
        }
        if let Some(t) = toasts.pop() {
            self.toast = Some(Toast::new(ctx, t));
        }
        for (target, e) in failed {
            if let Some(s) = self.mics.shares.remove(&target) {
                self.mic_failed(ctx, &s.host, s.asked, e);
            }
            self.mics.skip.insert(target);
        }

        if self.mics.last_check.is_some_and(|t| t.elapsed() < CHECK_INTERVAL) {
            return;
        }
        self.mics.last_check = Some(Instant::now());
        if self.cfg.ssh_mic == SshMic::Off {
            self.mics.shares.clear();
            return;
        }
        let present: HashSet<SshTarget> = self.panes.values().filter_map(|t| t.last_ssh_target()).collect();
        // No pane on the host any more: dropping the share stops it.
        self.mics.shares.retain(|t, _| present.contains(t));
        self.mics.skip.retain(|t| present.contains(t));
        if self.cfg.ssh_mic == SshMic::Auto {
            for target in present {
                if self.mics.shares.contains_key(&target) || self.mics.skip.contains(&target) {
                    continue;
                }
                // With sharing on, wait for the pane's login to finish (its connection is the
                // one nex's rides on), rather than fail while the password is being typed.
                if remote::sharing_enabled() && !remote::shared_connection_live(&target) {
                    continue;
                }
                self.start_mic(ctx, target, false);
            }
        }
    }

    /// Quitting: stop every share and give the hosts a moment to take nex's block back out
    /// of their `~/.asoundrc`.
    pub(super) fn stop_mics(&mut self) {
        let shares: Vec<MicShare> = self.mics.shares.drain().map(|(_, s)| s).collect();
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
            MicError::NeedsLogin | MicError::NotLinux | MicError::NoAlsa if !asked => return,
            MicError::NeedsLogin if remote::sharing_enabled() => {
                format!("Couldn't share the microphone: log out of {host} (type exit) and ssh back in, then share it again.")
            }
            MicError::NeedsLogin if remote::SHARING_SUPPORTED && !self.sharing_declined => {
                self.sharing_prompt = Some(SharingPrompt::Offer(host.to_string(), "Share the microphone again"));
                return;
            }
            MicError::NeedsLogin => format!("Couldn't share the microphone: {host} needs a password, and nex can't type one."),
            e => format!("Couldn't share the microphone with {host}: {e}"),
        };
        self.toast = Some(Toast::long(ctx, msg));
    }
}
