mod app;
mod cli;
mod config;
mod file_explorer;
mod platform;
mod render;
mod terminal;
mod ui;

fn main() -> eframe::Result {
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
    // Opened from Finder or the Dock (in `/`): the first shell starts at home, as in Terminal.
    if args.working_directory.is_none() && platform::opened_from_desktop() {
        if let Some(home) = platform::home_dir() {
            let _ = std::env::set_current_dir(home);
        }
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

    // On macOS the system draws its traffic lights over nex's own title bar.
    let overlay = platform::WINDOW.overlays_titlebar();
    let viewport = egui::ViewportBuilder::default()
        .with_title("nex")
        .with_app_id(args.app_id.as_deref().unwrap_or("nex"))
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([420.0, 260.0])
        .with_transparent(true)
        .with_decorations(cfg.window_decorations)
        .with_fullsize_content_view(overlay)
        .with_titlebar_shown(!overlay)
        .with_title_shown(!overlay);

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("nex", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, cfg, args)))))
}
