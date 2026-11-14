//! Font lookup through fontconfig (`fc-match`, `fc-list`).

use std::path::PathBuf;
use std::process::Command;

use crate::platform::{Family, FontFace, Weight};

/// Names must match fontconfig's family name exactly (`fc-list : family`). DejaVu Sans Mono
/// comes first since its symbols are drawn to the cell width; the Noto symbol fonts cover
/// Dingbats (✢ ✻ ✘), Misc Technical (⎯) and math operators (⧉).
pub const FALLBACK_FAMILIES: &[&str] =
    &["DejaVu Sans Mono", "Noto Sans Symbols2", "Noto Sans Symbols", "Noto Sans Math", "Noto Sans CJK SC"];

/// Resolve a fontconfig pattern to (file, family, index).
fn fc_match(pattern: &str) -> Option<(String, String, u32)> {
    let out = Command::new("fc-match").args(["-f", "%{file}\n%{family}\n%{index}", pattern]).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut lines = text.lines();
    let file = lines.next()?.to_string();
    let family = lines.next().unwrap_or_default().to_string();
    let index = lines.next().and_then(|i| i.trim().parse().ok()).unwrap_or(0);
    (!file.is_empty()).then_some((file, family, index))
}

/// The font for `family` in a style. A named family must really be installed: fc-match always
/// returns *something*, usually a proportional sans that would wreck the grid.
pub fn find_font(family: Family, weight: Weight, italic: bool) -> Option<FontFace> {
    let weight = match weight {
        Weight::Regular => "regular",
        Weight::Semibold => "semibold",
        Weight::Bold => "bold",
    };
    let slant = if italic { "italic" } else { "roman" };
    let name = match family {
        Family::Named(name) => name,
        Family::Monospace => "monospace",
        Family::SansSerif => "sans-serif",
    };
    let (file, matched, index) = fc_match(&format!("{name}:weight={weight}:slant={slant}"))?;
    if let Family::Named(name) = family {
        let wanted = name.to_lowercase();
        if !matched.split(',').any(|m| m.trim().to_lowercase() == wanted) {
            return None;
        }
    }
    Some(FontFace { path: PathBuf::from(file), index })
}

/// Installed "… Nerd Font Mono" families, sorted.
pub fn nerd_mono_families() -> Vec<String> {
    let Some(out) = Command::new("fc-list").args([":", "family"]).output().ok() else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut families: Vec<String> = text
        .lines()
        .flat_map(|l| l.split(','))
        .map(|f| f.trim().to_string())
        .filter(|f| f.ends_with("Nerd Font Mono"))
        .collect();
    families.sort();
    families.dedup();
    families
}
