//! Font discovery via fontconfig (`fc-match`) and registration with egui.

use std::process::Command;
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

use crate::config::Config;
use crate::render::{FONT_BOLD, FONT_BOLD_ITALIC, FONT_ITALIC, FONT_REGULAR};

/// Resolve a fontconfig pattern to (file, family).
fn fc_match(pattern: &str) -> Option<(String, String)> {
    let out = Command::new("fc-match").args(["-f", "%{file}\n%{family}", pattern]).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut lines = text.lines();
    let file = lines.next()?.to_string();
    let family = lines.next().unwrap_or_default().to_string();
    (!file.is_empty()).then_some((file, family))
}

fn load(pattern: &str) -> Option<FontData> {
    let (file, _) = fc_match(pattern)?;
    std::fs::read(file).ok().map(FontData::from_owned)
}

/// JetBrains Mono patched with Nerd Font icons, compiled into the binary (like Ghostty's
/// built-in font) so nvim / starship / lazygit icons work with no fonts installed.
/// License: SIL OFL 1.1, see assets/fonts/OFL.txt.
const EMBEDDED_FAMILY: &str = "JetBrainsMonoNL Nerd Font Mono";
const EMBEDDED: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/JetBrainsMonoNLNerdFontMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMonoNLNerdFontMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMonoNLNerdFontMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMonoNLNerdFontMono-BoldItalic.ttf"),
];
const EMBEDDED_KEY: &str = "embedded-nerd-regular";

/// The first installed "… Nerd Font Mono" family, preferring JetBrains Mono.
fn installed_nerd_mono() -> Option<String> {
    let out = Command::new("fc-list").args([":", "family"]).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut families: Vec<String> = text
        .lines()
        .flat_map(|l| l.split(','))
        .map(|f| f.trim().to_string())
        .filter(|f| f.ends_with("Nerd Font Mono"))
        .collect();
    families.sort();
    families.dedup();
    families
        .iter()
        .find(|f| f.starts_with("JetBrainsMono"))
        .or_else(|| families.first())
        .cloned()
}

/// File for `family` in `style`, but only if fontconfig really has that family (fc-match
/// always returns *something*, usually a proportional sans that would wreck the grid).
fn family_file(family: &str, style: &str) -> Option<String> {
    let (file, matched) = fc_match(&format!("{family}{style}"))?;
    let wanted = family.to_lowercase();
    let ok = family == "monospace" || matched.split(',').any(|m| m.trim().to_lowercase() == wanted);
    ok.then_some(file)
}

/// Which system family to use for the terminal grid; `None` means the embedded font.
fn resolve_family(cfg: &Config) -> Option<String> {
    let requested = cfg.font_family.trim();
    if requested.is_empty() {
        return installed_nerd_mono();
    }
    if family_file(requested, "").is_some() {
        return Some(requested.to_string());
    }
    eprintln!("nex: font-family `{requested}` not found; using built-in {EMBEDDED_FAMILY}");
    None
}

pub fn install(ctx: &egui::Context, cfg: &Config) {
    let mut defs = FontDefinitions::default();
    let family = resolve_family(cfg);
    defs.font_data.insert(EMBEDDED_KEY.into(), Arc::new(FontData::from_static(EMBEDDED[0])));

    let fallbacks = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let styles = [
        (FONT_REGULAR, ":weight=regular:slant=roman"),
        (FONT_BOLD, ":weight=bold:slant=roman"),
        (FONT_ITALIC, ":weight=regular:slant=italic"),
        (FONT_BOLD_ITALIC, ":weight=bold:slant=italic"),
    ];
    for (i, (name, style)) in styles.into_iter().enumerate() {
        let data = match &family {
            Some(f) => family_file(f, style).and_then(|file| std::fs::read(file).ok()).map(FontData::from_owned),
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
        // Symbols & CJK coverage from the system, when available.
        for extra in ["Noto Sans Symbols 2", "Noto Sans CJK SC"] {
            let key = format!("sys-{extra}");
            if !defs.font_data.contains_key(&key) {
                if let Some(file) = family_file(extra, "") {
                    if let Ok(bytes) = std::fs::read(file) {
                        defs.font_data.insert(key.clone(), Arc::new(FontData::from_owned(bytes)));
                    }
                }
            }
            if defs.font_data.contains_key(&key) {
                chain.push(key);
            }
        }
        defs.families.insert(FontFamily::Name(name.into()), chain);
    }

    // UI chrome uses the system sans font.
    let mut ui = Vec::new();
    let mut ui_bold = Vec::new();
    if let Some(data) = load("sans-serif:weight=regular") {
        defs.font_data.insert("ui-sans".into(), Arc::new(data));
        ui.push("ui-sans".to_string());
    }
    if let Some(data) = load("sans-serif:weight=semibold") {
        defs.font_data.insert("ui-sans-bold".into(), Arc::new(data));
        ui_bold.push("ui-sans-bold".to_string());
    }
    let prop = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    ui_bold.extend(ui.iter().cloned());
    ui_bold.extend(prop.iter().cloned());
    ui.extend(prop);
    defs.families.insert(FontFamily::Proportional, ui);
    defs.families.insert(FontFamily::Name("ui-bold".into()), ui_bold);

    ctx.set_fonts(defs);
}
