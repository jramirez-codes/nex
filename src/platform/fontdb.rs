//! Font lookup by scanning the system's font folders, for OSes without fontconfig.
//! The backend names its default sans-serif and monospace families (`SANS_SERIF`, `MONOSPACE`).

use std::sync::OnceLock;

use fontdb::{Database, Query, Source, Stretch, Style};

use super::backend::{MONOSPACE, SANS_SERIF};
use super::{Family, FontFace, Weight};

/// Scanned once; a few dozen milliseconds on a typical system.
fn db() -> &'static Database {
    static DB: OnceLock<Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family(SANS_SERIF);
        db.set_monospace_family(MONOSPACE);
        db
    })
}

/// The installed family whose name matches `name`, ignoring case, with its exact spelling.
fn installed_family(db: &Database, name: &str) -> Option<String> {
    db.faces().flat_map(|f| f.families.iter()).map(|(n, _)| n).find(|n| n.eq_ignore_ascii_case(name)).cloned()
}

/// The font for `family` in a style; a named family must be installed.
pub fn find_font(family: Family, weight: Weight, italic: bool) -> Option<FontFace> {
    let db = db();
    let named;
    let families = [match family {
        Family::Named(name) => {
            named = installed_family(db, name)?;
            fontdb::Family::Name(&named)
        }
        Family::Monospace => fontdb::Family::Monospace,
        Family::SansSerif => fontdb::Family::SansSerif,
    }];
    let weight = match weight {
        Weight::Regular => fontdb::Weight::NORMAL,
        Weight::Semibold => fontdb::Weight::SEMIBOLD,
        Weight::Bold => fontdb::Weight::BOLD,
    };
    let style = if italic { Style::Italic } else { Style::Normal };
    let id = db.query(&Query { families: &families, weight, stretch: Stretch::Normal, style })?;
    let face = db.face(id)?;
    match &face.source {
        Source::File(path) => Some(FontFace { path: path.clone(), index: face.index }),
        _ => None,
    }
}

/// Installed "… Nerd Font Mono" families, sorted.
pub fn nerd_mono_families() -> Vec<String> {
    let mut families: Vec<String> = db()
        .faces()
        .flat_map(|f| f.families.iter().map(|(n, _)| n.clone()))
        .filter(|n| n.ends_with("Nerd Font Mono"))
        .collect();
    families.sort();
    families.dedup();
    families
}
