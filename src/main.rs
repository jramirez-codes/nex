mod app;
mod glide;
mod theme;
mod boxdraw;
mod chrome;
mod config;
mod files;
mod fonts;
mod input;
mod layout;
mod render;
mod search;
mod terminal;

fn main() -> eframe::Result {
    let cfg = config::Config::load();

    let viewport = egui::ViewportBuilder::default()
        .with_title("nex")
        .with_app_id("nex")
        .with_inner_size([1100.0, 700.0])
        .with_min_inner_size([420.0, 260.0])
        .with_transparent(true)
        .with_decorations(cfg.window_decorations);

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("nex", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, cfg)))))
}
