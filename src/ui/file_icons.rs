//! Icons for files by type, drawn with the Nerd Font glyphs nex embeds (so they work with
//! nothing installed): a colored glyph per language or kind of file, and small marks for the
//! well-known folders.
//!
//! Icons are sized by their width, which takes knowing how wide the font draws each one: that
//! is measured once per glyph and kept (`width_ratio`), not laid out again for every icon of
//! every frame. The window chrome's codicons are sized and centered by their ink instead
//! (`paint_glyph_ink`), so icons the font places differently in their cells still line up; that
//! ink is measured once per glyph too (`ink_ratio`).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::{Align2, Color32, Context, FontFamily, FontId, Galley, Id, Painter, Pos2, Rect, Vec2};

use crate::render::FONT_REGULAR;

const IMAGE: Color32 = Color32::from_rgb(0xc7, 0x8b, 0xf0);
const VIDEO: Color32 = Color32::from_rgb(0xf0, 0x75, 0x8a);
const AUDIO: Color32 = Color32::from_rgb(0xf5, 0xa3, 0x5c);
const ARCHIVE: Color32 = Color32::from_rgb(0xe0, 0xb8, 0x5c);
const PDF: Color32 = Color32::from_rgb(0xf0, 0x6b, 0x6b);
const DOC: Color32 = Color32::from_rgb(0x5b, 0x9c, 0xf0);
const SHEET: Color32 = Color32::from_rgb(0x4f, 0xbf, 0x7a);
const SLIDES: Color32 = Color32::from_rgb(0xf0, 0x8a, 0x4b);
const PLAIN: Color32 = Color32::from_rgb(0xa9, 0xb1, 0xc6);
const CONFIG: Color32 = Color32::from_rgb(0x9a, 0xa5, 0xb1);
const SHELL: Color32 = Color32::from_rgb(0x89, 0xe0, 0x51);

/// The glyph and color for a file called `name`, if it's a kind nex knows. `executable`
/// files with no better match get a terminal glyph.
pub fn file_glyph(name: &str, executable: bool) -> Option<(char, Color32)> {
    by_name(name).or_else(|| by_extension(name)).or_else(|| executable.then_some(('\u{f018d}', SHELL)))
}

fn by_name(name: &str) -> Option<(char, Color32)> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "dockerfile" | "containerfile" | "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => {
            ('\u{f0868}', Color32::from_rgb(0x24, 0x96, 0xed))
        }
        "makefile" | "gnumakefile" | "cmakelists.txt" | "justfile" | "meson.build" => ('\u{f0493}', CONFIG),
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitconfig" | ".gitkeep" => ('\u{f02a2}', Color32::from_rgb(0xf1, 0x4e, 0x32)),
        "license" | "license.md" | "license.txt" | "copying" | "unlicense" => ('\u{f0fc3}', Color32::from_rgb(0xd0, 0xbf, 0x41)),
        "cargo.toml" | "cargo.lock" => ('\u{f1617}', Color32::from_rgb(0xde, 0xa5, 0x84)),
        "package.json" | "package-lock.json" | ".npmrc" => ('\u{f06f7}', Color32::from_rgb(0xe0, 0x4b, 0x4b)),
        "readme" | "readme.md" | "readme.txt" | "readme.rst" => ('\u{f00bd}', Color32::from_rgb(0x7f, 0xb4, 0xff)),
        ".env" | ".editorconfig" | ".bashrc" | ".zshrc" | ".profile" => ('\u{f0493}', CONFIG),
        _ => return None,
    })
}

fn by_extension(name: &str) -> Option<(char, Color32)> {
    let (_, ext) = name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty())?;
    let rgb = Color32::from_rgb;
    Some(match ext.to_lowercase().as_str() {
        "rs" => ('\u{f1617}', rgb(0xde, 0xa5, 0x84)),
        "py" | "pyi" | "pyw" | "ipynb" => ('\u{f0320}', rgb(0x5a, 0x9f, 0xd4)),
        "js" | "mjs" | "cjs" => ('\u{f031e}', rgb(0xf1, 0xe0, 0x5a)),
        "ts" | "mts" | "cts" => ('\u{f06e6}', rgb(0x4a, 0x9a, 0xe8)),
        "jsx" | "tsx" => ('\u{f0708}', rgb(0x61, 0xda, 0xfb)),
        "go" => ('\u{f07d3}', rgb(0x00, 0xad, 0xd8)),
        "c" | "h" => ('\u{f0671}', rgb(0x65, 0x9a, 0xd2)),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => ('\u{f0672}', rgb(0x65, 0x9a, 0xd2)),
        "java" => ('\u{f0b37}', rgb(0xe7, 0x6f, 0x00)),
        "kt" | "kts" => ('\u{f1219}', rgb(0xa9, 0x7b, 0xff)),
        "swift" => ('\u{f06e5}', rgb(0xf0, 0x51, 0x38)),
        "rb" => ('\u{f0d2d}', rgb(0xe0, 0x4a, 0x3f)),
        "php" => ('\u{f031f}', rgb(0x8f, 0x93, 0xd2)),
        "lua" => ('\u{f08b1}', rgb(0x51, 0xa0, 0xcf)),
        "hs" | "lhs" => ('\u{f0c92}', rgb(0x8f, 0x7c, 0xc3)),
        "cs" => ('\u{f031b}', rgb(0x9b, 0x4f, 0xc0)),
        "r" => ('\u{f07d4}', rgb(0x4a, 0x8a, 0xd8)),
        "nix" => ('\u{f1105}', rgb(0x7e, 0xba, 0xe4)),
        "vue" => ('\u{f0844}', rgb(0x41, 0xb8, 0x83)),
        "html" | "htm" => ('\u{f031d}', rgb(0xe3, 0x4c, 0x26)),
        "css" => ('\u{f031c}', rgb(0x8a, 0x6f, 0xd0)),
        "scss" | "sass" => ('\u{f07ec}', rgb(0xcd, 0x67, 0x99)),
        "json" | "jsonc" | "json5" => ('\u{f0626}', rgb(0xcb, 0xcb, 0x41)),
        "toml" => ('\u{e6b2}', rgb(0xc4, 0x6a, 0x3c)),
        "yaml" | "yml" => ('\u{e8eb}', rgb(0xcb, 0x5f, 0x5f)),
        "xml" | "plist" => ('\u{f05c0}', rgb(0xe3, 0x79, 0x33)),
        "ini" | "conf" | "cfg" | "config" | "desktop" | "service" => ('\u{f0493}', CONFIG),
        "md" | "markdown" | "mdx" => ('\u{f0354}', rgb(0x7f, 0xb4, 0xff)),
        "txt" | "log" | "rst" | "text" => ('\u{f09ee}', PLAIN),
        "sh" | "bash" | "zsh" | "fish" | "nu" | "ps1" | "bat" | "cmd" => ('\u{f1183}', SHELL),
        "sql" | "db" | "sqlite" | "sqlite3" => ('\u{f01bc}', rgb(0xdc, 0xb6, 0x7a)),
        "svg" => ('\u{f0721}', rgb(0xff, 0xb1, 0x3b)),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "heic" | "avif" | "xcf" | "psd" | "kra" => ('\u{f021f}', IMAGE),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "wmv" | "flv" => ('\u{f022b}', VIDEO),
        "mp3" | "flac" | "wav" | "ogg" | "oga" | "m4a" | "opus" | "aac" | "wma" => ('\u{f0223}', AUDIO),
        "zip" | "tar" | "gz" | "tgz" | "xz" | "txz" | "bz2" | "zst" | "7z" | "rar" | "deb" | "rpm" | "dmg" | "jar" | "whl" | "crate" => {
            ('\u{f05c4}', ARCHIVE)
        }
        "iso" | "img" => ('\u{f05ee}', ARCHIVE),
        "pdf" => ('\u{f0226}', PDF),
        "doc" | "docx" | "odt" | "rtf" | "pages" => ('\u{f022c}', DOC),
        "xls" | "xlsx" | "ods" | "numbers" => ('\u{f021b}', SHEET),
        "csv" | "tsv" => ('\u{f0218}', SHEET),
        "ppt" | "pptx" | "odp" | "key" => ('\u{f0227}', SLIDES),
        "ttf" | "otf" | "woff" | "woff2" | "ttc" => ('\u{f06d6}', PLAIN),
        "lock" => ('\u{f033e}', CONFIG),
        "pem" | "crt" | "cer" | "pub" | "gpg" | "asc" => ('\u{f0306}', rgb(0xe5, 0xc0, 0x7b)),
        "exe" | "msi" | "appimage" | "app" | "apk" => ('\u{f08c6}', SHELL),
        _ => return None,
    })
}

/// A small mark drawn on well-known folders: the home folder's Desktop, Documents,
/// Downloads, Pictures, Music and Videos (`in_home`), and `.git` or `node_modules` anywhere.
pub fn folder_mark(name: &str, in_home: bool) -> Option<char> {
    let home_mark = match name {
        "Desktop" => Some('\u{f0379}'),
        "Documents" => Some('\u{f0219}'),
        "Downloads" => Some('\u{f01da}'),
        "Pictures" => Some('\u{f02e9}'),
        "Music" => Some('\u{f0387}'),
        "Videos" | "Movies" => Some('\u{f0567}'),
        _ => None,
    };
    match name {
        ".git" => Some('\u{f02a2}'),
        "node_modules" => Some('\u{f0399}'),
        _ => home_mark.filter(|_| in_home),
    }
}

/// The link mark drawn on symlinks.
pub const LINK_MARK: char = '\u{f0337}';

/// Draw glyph `ch` about `width` wide, centered on `center`. The Nerd Font is a monospace
/// font, so every icon is one cell wide; the font size is picked to make that cell `width`.
pub fn paint_glyph(p: &Painter, center: Pos2, width: f32, ch: char, color: Color32) {
    let family = grid_family();
    let size = glyph_font_size(p.ctx(), &family, width, ch);
    p.text(center, Align2::CENTER_CENTER, ch, FontId::new(size, family), color);
}

/// The grid's regular font, where the icons come from. Made once: naming a family allocates,
/// cloning one doesn't.
fn grid_family() -> FontFamily {
    static FAMILY: OnceLock<FontFamily> = OnceLock::new();
    FAMILY.get_or_init(|| FontFamily::Name(FONT_REGULAR.into())).clone()
}

/// The font size at which `family` draws `ch` `width` wide.
fn glyph_font_size(ctx: &Context, family: &FontFamily, width: f32, ch: char) -> f32 {
    let ratio = width_ratio(ctx, family, ch);
    if ratio > 0.0 { width / ratio } else { width }
}

/// How wide each glyph is as a share of its font size, per family, as measured so far. Kept
/// in the context's memory, so each context has its own (their fonts can differ).
#[derive(Clone, Default)]
struct WidthRatios(HashMap<FontFamily, HashMap<char, f32>>);

/// The size the widths are measured at. Widths scale with the size, so any would do.
const PROBE_SIZE: f32 = 16.0;

/// How wide `family` draws `ch`, as a share of the font size. Measured the first time and
/// remembered until the fonts change ([`fonts_changed`]).
fn width_ratio(ctx: &Context, family: &FontFamily, ch: char) -> f32 {
    // Looked up in place: reading a value out of the context's memory would clone the maps.
    let known = ctx.data_mut(|d| d.get_temp_mut_or_default::<WidthRatios>(Id::NULL).0.get(family).and_then(|of| of.get(&ch)).copied());
    if let Some(ratio) = known {
        return ratio;
    }
    let ratio = ctx.fonts_mut(|f| f.glyph_width(&FontId::new(PROBE_SIZE, family.clone()), ch)) / PROBE_SIZE;
    ctx.data_mut(|d| d.get_temp_mut_or_default::<WidthRatios>(Id::NULL).0.entry(family.clone()).or_default().insert(ch, ratio));
    ratio
}

/// Draw glyph `ch` from `family` by its ink, not its cell: sized so the ink's larger side is
/// `side` (whole pixels), with the ink's center on `center` and its bitmap on the pixel grid.
/// Glyphs drawn this way at the same `side` line up, however differently the font places them
/// in their cells.
pub fn paint_glyph_ink(p: &Painter, family: FontFamily, center: Pos2, side: f32, ch: char, color: Color32) {
    let (galley, origin) = ink_placement(p.ctx(), family, center, side, ch);
    p.galley(origin, galley, color);
}

/// The layout [`paint_glyph_ink`] draws `ch` with, and where its top left goes.
///
/// The size comes from the ink measured once ([`ink_ratio`]). The position comes from this
/// layout's own bitmap: at small sizes the font's baseline is rounded to a pixel, which moves
/// the ink by up to a pixel from where the measurement would put it. The layout is the one
/// painting needs anyway (and egui keeps it from frame to frame), so this costs nothing more.
pub(super) fn ink_placement(ctx: &Context, family: FontFamily, center: Pos2, side: f32, ch: char) -> (Arc<Galley>, Pos2) {
    let ppp = ctx.pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let ink = ink_ratio(ctx, &family, ch);
    let larger = ink.width().max(ink.height());
    let size = if larger > 0.0 { snap(side).max(1.0 / ppp) / larger } else { side };
    let galley = ctx.fonts_mut(|f| f.layout_no_wrap(ch.to_string(), FontId::new(size, family), Color32::PLACEHOLDER));
    let bounds = galley.mesh_bounds;
    let ink_center = if bounds.is_positive() { bounds.center() } else { galley.rect.center() };
    let origin = center - ink_center.to_vec2();
    (galley, Pos2::new(snap(origin.x), snap(origin.y)))
}

/// Where each glyph's ink lies, as a share of the font size from the top left of its layout,
/// per family, as measured so far. Kept beside [`WidthRatios`], and forgotten with them.
#[derive(Clone, Default)]
struct InkRatios(HashMap<FontFamily, HashMap<char, Rect>>);

/// The size ink is measured at. A glyph's bitmap is whole pixels, so its edges are off by up
/// to a pixel: large enough that this is a sliver of the small sizes icons are drawn at.
const INK_PROBE_SIZE: f32 = 8.0 * PROBE_SIZE;

/// The ink rect of `ch` in `family`, as a share of the font size: laid out once and
/// remembered until the fonts change ([`fonts_changed`]).
pub(super) fn ink_ratio(ctx: &Context, family: &FontFamily, ch: char) -> Rect {
    let known = ctx.data_mut(|d| d.get_temp_mut_or_default::<InkRatios>(Id::NULL).0.get(family).and_then(|of| of.get(&ch)).copied());
    if let Some(ink) = known {
        return ink;
    }
    let galley = ctx.fonts_mut(|f| f.layout_no_wrap(ch.to_string(), FontId::new(INK_PROBE_SIZE, family.clone()), Color32::WHITE));
    let bounds = galley.mesh_bounds;
    let ink = if bounds.is_positive() {
        Rect::from_min_max((bounds.min.to_vec2() / INK_PROBE_SIZE).to_pos2(), (bounds.max.to_vec2() / INK_PROBE_SIZE).to_pos2())
    } else {
        Rect::from_min_size(Pos2::ZERO, Vec2::ZERO)
    };
    ctx.data_mut(|d| d.get_temp_mut_or_default::<InkRatios>(Id::NULL).0.entry(family.clone()).or_default().insert(ch, ink));
    ink
}

/// Forget the measured glyph widths and ink: call when `ctx` is given new fonts.
pub fn fonts_changed(ctx: &Context) {
    ctx.data_mut(|d| {
        d.remove_temp::<WidthRatios>(Id::NULL);
        d.remove_temp::<InkRatios>(Id::NULL);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_by_name_extension_and_mode() {
        assert_eq!(file_glyph("main.rs", false).map(|g| g.0), Some('\u{f1617}'));
        assert_eq!(file_glyph("Cargo.toml", false).map(|g| g.0), Some('\u{f1617}'));
        assert_eq!(file_glyph("PHOTO.JPG", false).map(|g| g.1), Some(IMAGE));
        assert_eq!(file_glyph("Dockerfile", false).map(|g| g.0), Some('\u{f0868}'));
        assert_eq!(file_glyph("build", true).map(|g| g.0), Some('\u{f018d}'));
        assert_eq!(file_glyph("notes", false), None);
        // A leading dot isn't an extension.
        assert_eq!(file_glyph(".rs", false), None);
        assert_eq!(folder_mark("Downloads", true), Some('\u{f01da}'));
        assert_eq!(folder_mark("Downloads", false), None);
        assert_eq!(folder_mark(".git", false), Some('\u{f02a2}'));
    }

    fn nerd_ctx() -> egui::Context {
        let font = include_bytes!("../../assets/fonts/JetBrainsMonoNLNerdFontMono-Regular.ttf");
        let mut defs = egui::FontDefinitions::default();
        defs.font_data.insert("nerd".into(), std::sync::Arc::new(egui::FontData::from_static(font)));
        defs.families.insert(grid_family(), vec!["nerd".into()]);
        let ctx = Context::default();
        ctx.set_fonts(defs);
        let _ = ctx.run(Default::default(), |_| {});
        ctx
    }

    fn measured(ctx: &Context) -> usize {
        ctx.data(|d| d.get_temp::<WidthRatios>(Id::NULL)).map_or(0, |r| r.0.values().map(HashMap::len).sum())
    }

    /// The size an icon is drawn at is the one the per-icon probe layout used to give, and
    /// each glyph is measured once however often and at whatever size it is drawn.
    #[test]
    fn icon_size_matches_a_probe_layout_and_is_measured_once() {
        let ctx = nerd_ctx();
        let family = grid_family();
        for ch in ['\u{f1617}', LINK_MARK, '\u{e6b2}'] {
            for width in [9.0_f32, 12.5, 15.0, 31.0] {
                let probe = ctx.fonts_mut(|f| f.layout_no_wrap(ch.to_string(), FontId::new(width, family.clone()), Color32::WHITE)).size().x;
                let by_probe = width * width / probe;
                let size = glyph_font_size(&ctx, &family, width, ch);
                // A layout's size is rounded to 1/32 of a point; the measured width is not.
                assert!((size - by_probe).abs() < by_probe * 0.01, "U+{:x} at {width}: {size} vs {by_probe}", ch as u32);
            }
        }
        assert_eq!(measured(&ctx), 3, "one measurement per glyph, not per call");
        // New fonts can draw the glyphs at other widths.
        fonts_changed(&ctx);
        assert_eq!(measured(&ctx), 0);
    }

    /// Every glyph above must exist in the embedded font, or it would draw as a box.
    #[test]
    fn glyphs_are_in_the_embedded_font() {
        let font = include_bytes!("../../assets/fonts/JetBrainsMonoNLNerdFontMono-Regular.ttf");
        let mut defs = egui::FontDefinitions::empty();
        defs.font_data.insert("nerd".into(), std::sync::Arc::new(egui::FontData::from_static(font)));
        defs.families.insert(FontFamily::Monospace, vec!["nerd".into()]);
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        let _ = ctx.run(Default::default(), |_| {});
        // Every glyph in this file, and the codicons the window chrome draws.
        let codicons = include_str!("icons.rs").lines().filter(|l| l.starts_with("const COD_")).collect::<Vec<_>>();
        assert!(codicons.len() >= 9, "only {} codicons found", codicons.len());
        let src = format!("{}\n{}", include_str!("file_icons.rs"), codicons.join("\n"));
        let mut checked = 0;
        for part in src.split("'\\u{").skip(1) {
            let hex = &part[..part.find('}').unwrap()];
            let ch = char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap();
            let has = ctx.fonts_mut(|f| f.has_glyph(&FontId::monospace(12.0), ch));
            assert!(has, "U+{hex} is missing from the embedded font");
            checked += 1;
        }
        assert!(checked > 50, "only {checked} glyphs found");
    }
}
