//! nex: a GPU-accelerated terminal emulator with tabs, split panes, a file explorer and ssh
//! that follows you. This file builds the window and hands the frame loop to [`app::App`]; the
//! desktop program (`main.rs`) parses the command line first, and the Android app
//! (`nex-android/`) and the iPhone app (`nex-ios/`) start here through [`android_main`] and
//! `ios_main`. See ARCHITECTURE.md for the map of
//! the modules below.

mod agents;
mod app;
mod cli;
mod config;
mod file_explorer;
mod git_diff;
mod layout;
mod platform;
mod remote;
mod render;
mod session;
mod terminal;
mod ui;

/// The desktop program: `nex [OPTIONS]` (see `cli.rs`).
pub fn main() -> eframe::Result {
    platform::tune_allocator();
    let args = match cli::Args::parse(std::env::args_os().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprint!("nex: {e}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    if args.help {
        print!("{}", cli::USAGE);
        return Ok(());
    }
    if args.version {
        println!("nex {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(path) = &args.follow_subagent {
        let found = (!path.as_os_str().is_empty()).then_some(path.as_path());
        let why = "No transcript for this sub-agent: it may have finished a while ago, or its agent has ended.";
        let code = nex_host::transcript::run(found, why).unwrap_or(1);
        std::process::exit(code);
    }
    if args.file_manager_service {
        if let Err(e) = platform::file_manager_service(&args.for_new_window()) {
            eprintln!("nex: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    run(args, |builder| builder)
}

/// The Android app's entry point, called by `nex-android` from its `android_main`. There is no
/// command line, and nothing runs on the phone but the window: every pane works on an ssh
/// host (see `platform/android/`).
#[cfg(target_os = "android")]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    platform::android_setup(&app);
    if let Err(e) = run(cli::Args::default(), move |builder| builder.with_android_app(app)) {
        eprintln!("nex: {e}");
    }
}

/// The iPhone app's entry point, called by `nex-ios` from `main` on the main thread. Like
/// Android, it has no command line and every pane works on an ssh host (see `platform/ios/`).
/// UIKit runs the app from here on: this never returns.
#[cfg(target_os = "ios")]
pub fn ios_main() {
    platform::ios_setup();
    if let Err(e) = run(cli::Args::default(), |builder| builder) {
        eprintln!("nex: {e}");
    }
}

type EventLoopBuilder = winit::event_loop::EventLoopBuilder<eframe::UserEvent>;

/// Open the window and run until it closes. `event_loop` finishes the event loop's builder
/// (on Android it is handed the activity).
fn run(args: cli::Args, event_loop: impl FnOnce(&mut EventLoopBuilder) -> &mut EventLoopBuilder) -> eframe::Result {
    // Opened from Finder or the Dock (in `/`): the first shell starts at home, as in Terminal.
    if args.working_directory.is_none() && platform::opened_from_desktop()
        && let Some(home) = platform::home_dir() {
            let _ = std::env::set_current_dir(home);
        }
    if let Some(dir) = &args.working_directory {
        // The first shell starts in nex's own working directory.
        if let Err(e) = std::env::set_current_dir(dir) {
            eprintln!("nex: --working-directory {}: {e}", dir.display());
        }
    }
    if let Some(path) = &args.config {
        config::set_path(path.clone());
    }
    let cfg = config::Config::load();
    // Only one window owns the session: a second plain `nex` starts fresh, as `--no-restore`.
    let session_lock = if args.restores_session() { session::try_lock() } else { None };
    let session = if session_lock.is_some() && cfg.restore_session { session::load() } else { None };

    // The first shell starts now, so it gets going while the window opens: in the last
    // session's first pane when it is coming back (none if that session had no terminals).
    // Where panes are on a host (Android, `--remote`), there is nothing to start early.
    let early_cwd = match &session {
        _ if !platform::LOCAL_SHELLS || args.remote.is_some() => None,
        Some(s) => app::restore::first_shell_dir(s).map(Some),
        None => Some(std::env::current_dir().ok()),
    };
    let early = early_cwd.and_then(|cwd| match terminal::Terminal::spawn_early(&cfg, args.command.clone(), cwd) {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("nex: failed to spawn shell: {e}");
            None
        }
    });

    // On macOS the system draws its traffic lights over nex's own title bar.
    let overlay = platform::WINDOW.overlays_titlebar();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("nex")
        .with_app_id(args.app_id.as_deref().unwrap_or("nex"))
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size(platform::WINDOW.min_size)
        .with_transparent(platform::WINDOW.transparent)
        .with_decorations(cfg.window_decorations)
        .with_fullsize_content_view(overlay)
        .with_titlebar_shown(!overlay)
        .with_title_shown(!overlay);
    if let Some(icon) = platform::app_icon() {
        viewport = viewport.with_icon(icon);
    }

    // nex's own event loop rather than `run_native`'s, so drag and drop can set up on its
    // connection before eframe opens the window (see `platform::Dnd::prepare`).
    let mut builder = winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event();
    let event_loop = event_loop(&mut builder).build()?;
    platform::Dnd::prepare(&event_loop);
    let vsync = platform::vsync(&event_loop);
    let options = eframe::NativeOptions { viewport, vsync, ..Default::default() };
    let eframe_app = eframe::create_native("nex", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, cfg, args, early, session, session_lock)))), &event_loop);
    platform::WinitApp::new(eframe_app, !vsync).run(event_loop)
}
