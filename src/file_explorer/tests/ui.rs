//! Drives an explorer pane through egui with synthetic pointer events, finding widgets by
//! their accessibility labels, to check what menus and buttons actually do.

use std::path::PathBuf;

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

use crate::file_explorer::FileExplorer;
use crate::file_explorer::tests::fixture;
use crate::ui::theme::Theme;

pub(in crate::file_explorer) struct Harness {
    pub(in crate::file_explorer) ctx: egui::Context,
    pub(in crate::file_explorer) b: FileExplorer,
    theme: Theme,
    /// Labeled widgets and where they are. Accessibility updates only carry what changed,
    /// so this keeps every node seen, newest first.
    pub(in crate::file_explorer) nodes: Vec<(String, Rect)>,
    seen: std::collections::HashMap<u64, (String, Rect, u64)>,
    time: f64,
    pass: u64,
    /// Modifier keys held down.
    pub(in crate::file_explorer) mods: Modifiers,
}

impl Harness {
    pub(in crate::file_explorer) fn new(dir: PathBuf) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        crate::render::fonts::install(&ctx, &crate::config::Config::default());
        let theme = Theme::from_config(&crate::config::Config::default());
        let mut h =
            Self { ctx, b: FileExplorer::new(dir), theme, nodes: Vec::new(), seen: Default::default(), time: 0.0, pass: 0, mods: Modifiers::NONE };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    pub(in crate::file_explorer) fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            time: Some(self.time),
            modifiers: self.mods,
            events,
            ..Default::default()
        };
        let (b, theme) = (&mut self.b, &self.theme);
        let out = self.ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                let rect = ui.max_rect();
                b.paint(ui, ui.painter(), rect, 1, theme);
            });
        });
        self.pass += 1;
        if let Some(update) = out.platform_output.accesskit_update {
            for (id, node) in update.nodes {
                if let (Some(label), Some(r)) = (node.label(), node.bounds()) {
                    let r = Rect::from_min_max(Pos2::new(r.x0 as f32, r.y0 as f32), Pos2::new(r.x1 as f32, r.y1 as f32));
                    self.seen.insert(id.0, (label.to_string(), r, self.pass));
                }
            }
        }
        let mut nodes: Vec<_> = self.seen.values().cloned().collect();
        nodes.sort_by_key(|n| std::cmp::Reverse(n.2));
        self.nodes = nodes.into_iter().map(|(l, r, _)| (l, r)).collect();
    }

    pub(in crate::file_explorer) fn click_at(&mut self, at: Pos2, button: PointerButton) {
        let press = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![press(true)]);
        self.frame(vec![press(false)]);
        self.frame(vec![]);
    }

    /// Press and release in one frame, as a quick click often arrives.
    pub(in crate::file_explorer) fn quick_click_at(&mut self, at: Pos2, button: PointerButton) {
        let press = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![press(true), press(false)]);
        self.frame(vec![]);
    }

    pub(in crate::file_explorer) fn find(&self, label: &str) -> Option<Rect> {
        self.nodes.iter().find(|(l, _)| l == label).map(|(_, r)| *r)
    }

    pub(in crate::file_explorer) fn click_label(&mut self, label: &str) {
        let r = self.find(label).unwrap_or_else(|| panic!("no “{label}” in {:?}", self.nodes.iter().map(|n| &n.0).collect::<Vec<_>>()));
        self.click_at(r.center(), PointerButton::Primary);
    }

    /// The center of row `i` of the list view.
    pub(in crate::file_explorer) fn row(&self, i: usize) -> Pos2 {
        Pos2::new(200.0, 30.0 + 4.0 + 24.0 * i as f32 + 12.0)
    }
}

#[test]
fn menu_delete_asks_first() {
    let dir = fixture("ui-delete");
    let mut h = Harness::new(dir.clone());
    h.click_at(h.row(3), PointerButton::Secondary);
    h.click_label("Delete…");
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert_eq!(h.b.confirm_delete.as_deref(), Some(&[dir.join("zeta.txt")][..]), "nodes: {:?}", h.nodes.iter().map(|n| &n.0).collect::<Vec<_>>());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn menu_properties_opens() {
    let dir = fixture("ui-props");
    let mut h = Harness::new(dir.clone());
    h.click_at(h.row(3), PointerButton::Secondary);
    h.click_label("Properties…");
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert!(h.b.properties.is_some(), "nodes: {:?}", h.nodes.iter().map(|n| &n.0).collect::<Vec<_>>());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn quick_clicks_in_the_menu() {
    let dir = fixture("ui-quick");
    let mut h = Harness::new(dir.clone());
    h.quick_click_at(h.row(3), PointerButton::Secondary);
    let r = h.find("Delete…").expect("menu open");
    h.quick_click_at(r.center(), PointerButton::Primary);
    h.frame(vec![]);
    assert!(h.b.confirm_delete.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A folder with enough files that the menu opens over other rows, and `file03.txt`
/// left-clicked (as people do) then right-clicked. Returns its path.
fn menu_over_rows(h: &mut Harness, dir: &std::path::Path) -> PathBuf {
    for i in 0..20 {
        std::fs::write(dir.join(format!("file{i:02}.txt")), "x").unwrap();
    }
    h.b.refresh();
    h.frame(vec![]);
    let target = h.b.entries.iter().position(|e| e.name == "file03.txt").unwrap();
    h.click_at(h.row(target), PointerButton::Primary);
    h.click_at(h.row(target), PointerButton::Secondary);
    dir.join("file03.txt")
}

#[test]
fn menu_delete_over_other_rows_asks_first() {
    let dir = fixture("ui-rows-delete");
    let mut h = Harness::new(dir.clone());
    let file = menu_over_rows(&mut h, &dir);
    h.click_label("Delete…");
    assert_eq!(h.b.confirm_delete.as_deref(), Some(&[file.clone()][..]));
    // The dialog stays up, and nothing is deleted until it's confirmed.
    for _ in 0..5 {
        h.frame(vec![]);
    }
    assert!(h.find("Delete").is_some() && h.find("Cancel").is_some() && file.exists());
    h.click_label("Delete");
    for _ in 0..10 {
        h.frame(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!file.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn menu_properties_over_other_rows() {
    let dir = fixture("ui-rows-props");
    let mut h = Harness::new(dir.clone());
    let file = menu_over_rows(&mut h, &dir);
    h.click_label("Properties…");
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert!(h.find("Close").is_some(), "no dialog: {:?}", h.nodes.iter().map(|n| &n.0).collect::<Vec<_>>());
    assert!(h.nodes.iter().any(|(l, _)| l.contains("file03.txt")) || h.b.properties.as_ref().is_some_and(|p| p.path() == file));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn menu_open_with_over_other_rows() {
    let dir = fixture("ui-rows-openwith");
    let mut h = Harness::new(dir.clone());
    // A stand-in app (`true` ignores the file) so the test doesn't start a real one.
    let app = crate::platform::AppChoice { name: "Stand-in".into(), command: "true".into(), default: true };
    let apps = std::sync::Arc::new(std::sync::Mutex::new(Some(vec![app])));
    h.b.open_with = Some(crate::file_explorer::input::edit::OpenWith { path: dir.join("file03.txt"), apps });
    menu_over_rows(&mut h, &dir);
    let r = h.find("Open With ⏵").expect("Open With in the menu");
    h.frame(vec![Event::PointerMoved(r.center())]);
    for _ in 0..3 {
        h.frame(vec![]);
    }
    h.click_label("Stand-in  (default)");
    let status = h.b.status.as_ref().map(|s| s.0.clone());
    assert_eq!(status.as_deref(), Some("Opening with Stand-in…"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn action_bar_delete_asks_first() {
    let dir = fixture("ui-bar-delete");
    let mut h = Harness::new(dir.clone());
    // Ctrl+click marks an item, which brings up the action bar.
    let at = h.row(3);
    h.mods = Modifiers::COMMAND;
    h.click_at(at, PointerButton::Primary);
    h.mods = Modifiers::NONE;
    h.frame(vec![]);
    h.click_label("Delete");
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert!(h.b.confirm_delete.is_some() && h.find("Cancel").is_some());
    assert!(dir.join("zeta.txt").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Dragging an item holds its own icon, at the size it has in the listing, where it was
/// grabbed; the picture handed to the compositor for outside the window matches it.
#[test]
fn drag_shows_the_items_icon_at_its_size() {
    let dir = fixture("drag-ghost");
    for view in [crate::file_explorer::View::List, crate::file_explorer::View::Grid] {
        let mut h = Harness::new(dir.clone());
        h.b.view = view;
        h.frame(vec![]);
        let layout = h.b.layout.expect("laid out");
        let s = crate::file_explorer::view::items::sizes(false, 1.0);
        let item = layout.item_rect(0, 0.0);
        let icon = match view {
            crate::file_explorer::View::List => crate::file_explorer::view::items::row_icon_rect(item, &s, false),
            crate::file_explorer::View::Grid => crate::file_explorer::view::items::tile_icon_rect(item, &s, h.b.entries[0].is_dir),
        };
        let at = icon.center() + Vec2::new(2.0, 1.0);
        let button = |pressed, pos| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        h.frame(vec![Event::PointerMoved(at)]);
        h.frame(vec![button(true, at)]);
        h.frame(vec![Event::PointerMoved(at + Vec2::new(30.0, 30.0))]);
        let drag = egui::DragAndDrop::payload::<crate::file_explorer::ExplorerDrag>(&h.ctx).expect("dragging");
        assert_eq!(drag.ghost.size, icon.size());
        assert_eq!(drag.ghost.grab, at - icon.min);

        let mut image = None;
        let _ = h.ctx.run(RawInput::default(), |ctx| image = crate::file_explorer::drag_image(ctx, &h.theme, &drag));
        let img = image.expect("an image");
        // The icon plus a pixel of margin, with the pointer where it was grabbed.
        assert!(img.size[0] as f32 >= icon.width() && img.size[0] as f32 <= icon.width() + 4.0, "{:?} vs {icon:?}", img.size);
        assert!((img.hotspot[0] - drag.ghost.grab.x.round() as i32).abs() <= 2, "{:?}", img.hotspot);
        let opaque = img.pixels.iter().filter(|p| p[3] > 200).count();
        assert!(opaque > img.pixels.len() / 4, "{opaque} of {} pixels drawn", img.pixels.len());
        if std::env::var_os("NEX_DUMP_GHOST").is_some() {
            for row in img.pixels.chunks(img.size[0]) {
                eprintln!(
                    "{}",
                    row.iter()
                        .map(|p| match p[3] {
                            0..=40 => ' ',
                            41..=200 => '+',
                            _ => '#',
                        })
                        .collect::<String>()
                );
            }
        }
        egui::DragAndDrop::clear_payload(&h.ctx);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Dragging a file onto a folder moves it, and the listing drops it once the move finishes.
#[test]
fn drag_onto_folder_moves_and_updates_listing() {
    let dir = fixture("drag-move");
    let mut h = Harness::new(dir.clone());
    let src_i = h.b.entries.iter().position(|e| e.name == "zeta.txt").unwrap();
    let dst_i = h.b.entries.iter().position(|e| e.name == "beta").unwrap();
    let (from, to) = (h.row(src_i), h.row(dst_i));
    let button = |pressed, pos| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.frame(vec![Event::PointerMoved(from)]);
    h.frame(vec![button(true, from)]);
    for k in 1..=10 {
        h.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 10.0))]);
    }
    h.frame(vec![button(false, to)]);
    for _ in 0..20 {
        h.frame(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(dir.join("beta/zeta.txt").exists() && !dir.join("zeta.txt").exists());
    assert!(!h.b.entries.iter().any(|e| e.name == "zeta.txt"));
    std::fs::remove_dir_all(dir).unwrap();
}
