//! Font setup for egui: the terminal grid's font (a system family or the embedded Nerd Font),
//! fallbacks for symbols and CJK, and the UI's sans-serif. Finding fonts on the system is the
//! platform's job (`platform::find_font`).

use std::fs::File;
use std::sync::{Arc, Mutex};

use egui::{FontData, FontDefinitions, FontFamily};

use crate::config::Config;
use crate::platform::{self, Family, FontFace, Weight};
use crate::render::{FONT_BOLD, FONT_BOLD_ITALIC, FONT_ICONS, FONT_ITALIC, FONT_REGULAR};
use crate::ui::paint;

/// Font files are memory-mapped, not read. egui uses borrowed font bytes as they are but
/// clones owned ones, so reading a file kept it in memory twice; a mapping costs nothing up
/// front and only pulls in the pages glyphs are drawn from (most of a 19 MB CJK font is never
/// touched). Mappings live for the rest of the process and are shared by path, so reloading
/// the config doesn't map a file again.
fn map_font(face: &FontFace) -> Option<FontData> {
    static MAPS: Mutex<Vec<(std::path::PathBuf, &'static [u8])>> = Mutex::new(Vec::new());
    let with_index = |bytes| FontData { index: face.index, ..FontData::from_static(bytes) };
    let mut maps = MAPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, bytes)) = maps.iter().find(|(p, _)| *p == face.path) {
        return Some(with_index(bytes));
    }
    let file = File::open(&face.path).ok()?;
    // SAFETY: the mapping is read-only and never unmapped. Like FreeType, this assumes font
    // files aren't truncated while nex is running.
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let bytes: &'static [u8] = Box::leak(Box::new(map));
    maps.push((face.path.clone(), bytes));
    Some(with_index(bytes))
}

fn load(family: Family, weight: Weight, italic: bool) -> Option<FontData> {
    map_font(&platform::find_font(family, weight, italic)?)
}

/// JetBrains Mono patched with Nerd Font icons, compiled into the binary (like Ghostty's
/// built-in font) so nvim / starship / lazygit icons work with no fonts installed.
/// License: SIL OFL 1.1, see assets/fonts/OFL.txt.
const EMBEDDED_FAMILY: &str = "JetBrainsMonoNL Nerd Font Mono";
const EMBEDDED: [&[u8]; 4] = [
    include_bytes!("../../assets/fonts/JetBrainsMonoNLNerdFontMono-Regular.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMonoNLNerdFontMono-Bold.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMonoNLNerdFontMono-Italic.ttf"),
    include_bytes!("../../assets/fonts/JetBrainsMonoNLNerdFontMono-BoldItalic.ttf"),
];
const EMBEDDED_KEY: &str = "embedded-nerd-regular";

/// The first installed "… Nerd Font Mono" family, preferring JetBrains Mono.
fn installed_nerd_mono() -> Option<String> {
    let families = platform::nerd_mono_families();
    families.iter().find(|f| f.starts_with("JetBrainsMono")).or_else(|| families.first()).cloned()
}

/// `monospace` names the system's default monospace font rather than a family.
fn family(name: &str) -> Family<'_> {
    if name.eq_ignore_ascii_case("monospace") { Family::Monospace } else { Family::Named(name) }
}

/// Which system family to use for the terminal grid; `None` means the embedded font.
fn resolve_family(cfg: &Config) -> Option<String> {
    let requested = cfg.font_family.trim();
    if requested.is_empty() {
        return installed_nerd_mono();
    }
    if platform::find_font(family(requested), Weight::Regular, false).is_some() {
        return Some(requested.to_string());
    }
    eprintln!("nex: font-family `{requested}` not found; using built-in {EMBEDDED_FAMILY}");
    None
}

/// The grid font's four styles (regular, bold, italic, bold italic), each `None` when the
/// family has no such face. `None` overall means the embedded font.
fn grid_faces(cfg: &Config) -> Option<[Option<FontData>; 4]> {
    let name = resolve_family(cfg)?;
    let family = family(&name);
    Some(std::thread::scope(|s| {
        GRID_STYLES.map(|(_, weight, italic)| s.spawn(move || load(family, weight, italic))).map(|h| h.join().ok().flatten())
    }))
}

const GRID_STYLES: [(&str, Weight, bool); 4] = [
    (FONT_REGULAR, Weight::Regular, false),
    (FONT_BOLD, Weight::Bold, false),
    (FONT_ITALIC, Weight::Regular, true),
    (FONT_BOLD_ITALIC, Weight::Bold, true),
];

pub fn install(ctx: &egui::Context, cfg: &Config) {
    // Every lookup runs at once. On Linux each one is an `fc-match` process that loads the
    // fontconfig cache (around 10 ms apiece), and run one after another they held the window
    // back by over 100 ms.
    let (grid, symbol_data, sans, sans_bold) = std::thread::scope(|s| {
        let grid = s.spawn(|| grid_faces(cfg));
        let symbols: Vec<_> = platform::FALLBACK_FAMILIES
            .iter()
            .map(|&extra| s.spawn(move || (extra, load(Family::Named(extra), Weight::Regular, false))))
            .collect();
        let sans = s.spawn(|| load(Family::SansSerif, Weight::Regular, false));
        let sans_bold = s.spawn(|| load(Family::SansSerif, Weight::Semibold, false));
        let join = |h: std::thread::ScopedJoinHandle<'_, Option<FontData>>| h.join().ok().flatten();
        (
            grid.join().ok().flatten(),
            symbols.into_iter().filter_map(|h| h.join().ok()).collect::<Vec<_>>(),
            join(sans),
            join(sans_bold),
        )
    });

    let mut defs = FontDefinitions::default();
    defs.font_data.insert(EMBEDDED_KEY.into(), Arc::new(FontData::from_static(EMBEDDED[0])));

    let fallbacks = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    // Symbols & CJK coverage from the system, when available.
    let mut symbols = Vec::new();
    for (extra, data) in symbol_data {
        if let Some(data) = data {
            let key = format!("sys-{extra}");
            defs.font_data.insert(key.clone(), Arc::new(data));
            symbols.push(key);
        }
    }
    let mut grid = grid;
    for (i, (name, _, _)) in GRID_STYLES.into_iter().enumerate() {
        let data = match &mut grid {
            Some(faces) => faces[i].take(),
            None => Some(FontData::from_static(EMBEDDED[i])),
        };
        let mut chain = Vec::new();
        if let Some(data) = data {
            defs.font_data.insert(name.into(), Arc::new(data));
            chain.push(name.to_string());
        }
        // Missing bold/italic falls back to the family's regular face.
        if name != FONT_REGULAR && defs.font_data.contains_key(FONT_REGULAR) {
            chain.push(FONT_REGULAR.to_string());
        }
        // Nerd Font icons are always available, whatever font the grid uses.
        chain.push(EMBEDDED_KEY.to_string());
        chain.extend(fallbacks.iter().cloned());
        chain.extend(symbols.iter().cloned());
        defs.families.insert(FontFamily::Name(name.into()), chain);
    }

    defs.families.insert(FontFamily::Name(FONT_ICONS.into()), vec![EMBEDDED_KEY.to_string()]);

    // UI chrome uses the system sans font.
    let mut ui = Vec::new();
    let mut ui_bold = Vec::new();
    if let Some(data) = sans {
        defs.font_data.insert("ui-sans".into(), Arc::new(data));
        ui.push("ui-sans".to_string());
    }
    if let Some(data) = sans_bold {
        defs.font_data.insert("ui-sans-bold".into(), Arc::new(data));
        ui_bold.push("ui-sans-bold".to_string());
    }
    let mut prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // Titles and file names carry whatever programs put in them: Claude Code's spinner
    // (braille, ✢ ✻ ✽), arrows, box drawing, Nerd Font icons. The sans font has few of those,
    // so the UI falls back to the same symbol fonts as the grid instead of drawing boxes.
    prop.push(EMBEDDED_KEY.to_string());
    prop.extend(symbols);
    ui_bold.extend(ui.iter().cloned());
    ui_bold.extend(prop.iter().cloned());
    ui.extend(prop);
    defs.families.insert(FontFamily::Proportional, ui);
    defs.families.insert(FontFamily::Name(paint::UI_BOLD.into()), ui_bold);

    ctx.set_fonts(defs);
}

#[cfg(test)]
mod tests {
    use egui::{FontFamily, FontId};

    /// Symbols Claude Code puts in the terminal title and prompt (spinner frames, braille
    /// spinner, status marks). Whatever the grid can draw, the UI (tab titles, title bar) must
    /// draw too, rather than showing boxes.
    #[test]
    fn ui_draws_the_symbols_the_grid_does() {
        let ctx = egui::Context::default();
        super::install(&ctx, &crate::config::Config::default());
        let _ = ctx.run(Default::default(), |_| {});
        let grid = FontId::new(14.0, FontFamily::Name(super::FONT_REGULAR.into()));
        let ui = [FontId::proportional(14.0), FontId::new(14.0, FontFamily::Name(super::paint::UI_BOLD.into()))];
        for ch in "·✢✳✶✻✽⏺⎿⏵↯⧉✔✘☐☒◯❯⎯⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏⠂⠐\u{f0337}".chars() {
            if !ctx.fonts_mut(|f| f.has_glyph(&grid, ch)) {
                continue;
            }
            for id in &ui {
                assert!(ctx.fonts_mut(|f| f.has_glyph(id, ch)), "{ch} (U+{:04X}) missing from {:?}", ch as u32, id.family);
            }
        }
    }
}
