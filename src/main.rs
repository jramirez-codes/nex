mod app;
mod cli;
mod config;
mod file_explorer;
mod render;
mod terminal;
mod ui;
mod wl_dnd;

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

    let viewport = egui::ViewportBuilder::default()
        .with_title("nex")
        .with_app_id(args.app_id.as_deref().unwrap_or("nex"))
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([420.0, 260.0])
        .with_transparent(true)
        .with_decorations(cfg.window_decorations);

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("nex", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, cfg, args)))))
}
