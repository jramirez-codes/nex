//! Explorer tests: the shared folder fixture and unit tests of the pane state (here), and UI
//! tests that drive a pane through egui (`ui.rs`).

mod ui;

use super::*;

pub(super) fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nex-files-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("beta")).unwrap();
    std::fs::create_dir_all(dir.join("Alpha")).unwrap();
    std::fs::create_dir_all(dir.join(".hidden_dir")).unwrap();
    std::fs::write(dir.join("zeta.txt"), "z").unwrap();
    std::fs::write(dir.join("Apple.md"), "apple").unwrap();
    std::fs::write(dir.join(".env"), "x").unwrap();
    dir
}

fn names(b: &FileExplorer) -> Vec<&str> {
    b.entries.iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn folders_first_then_case_insensitive() {
    let dir = fixture("sort");
    let b = FileExplorer::new(dir.clone());
    assert_eq!(names(&b), ["Alpha", "beta", "Apple.md", "zeta.txt"]);
    assert_eq!(b.entries[2].size, Some(5));
    assert_eq!(b.entries[0].size, None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hidden_files_toggle() {
    let dir = fixture("hidden");
    let mut b = FileExplorer::new(dir.clone());
    assert!(!names(&b).iter().any(|n| n.starts_with('.')));
    b.toggle_hidden();
    assert_eq!(names(&b), [".hidden_dir", "Alpha", "beta", ".env", "Apple.md", "zeta.txt"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reveal_item_selects_it_showing_hidden_files_if_needed() {
    let dir = fixture("reveal");
    let mut b = FileExplorer::new(dir.clone());
    b.reveal_item(&dir.join("zeta.txt"));
    assert_eq!(b.entries[b.selected].name, "zeta.txt");
    assert!(!b.show_hidden);
    b.reveal_item(&dir.join(".env"));
    assert!(b.show_hidden);
    assert_eq!(b.entries[b.selected].name, ".env");
    // Items outside this folder are left alone.
    b.reveal_item(&dir.join("beta/nothing"));
    assert_eq!(b.entries[b.selected].name, ".env");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn up_selects_the_folder_we_came_from() {
    let dir = fixture("up");
    let mut b = FileExplorer::new(dir.join("beta"));
    b.up();
    assert_eq!(b.dir, dir);
    assert_eq!(b.entries[b.selected].name, "beta");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn back_and_forward() {
    let dir = fixture("history");
    std::fs::create_dir(dir.join("beta/deep")).unwrap();
    let mut b = FileExplorer::new(dir.clone());
    assert!(!b.can_go_back());
    b.navigate(dir.join("beta"));
    b.navigate(dir.join("beta/deep"));
    b.go_back();
    assert_eq!(b.dir, dir.join("beta"));
    assert_eq!(b.entries[b.selected].name, "deep");
    b.go_back();
    assert_eq!(b.dir, dir);
    assert_eq!(b.entries[b.selected].name, "beta");
    assert!(!b.can_go_back());
    b.go_forward();
    assert_eq!(b.dir, dir.join("beta"));
    // Going somewhere new forgets the folders ahead.
    b.navigate(dir.join("Alpha"));
    assert!(!b.can_go_forward());
    b.go_back();
    assert_eq!(b.dir, dir.join("beta"));
    // Up counts as a step too.
    b.up();
    b.go_back();
    assert_eq!(b.dir, dir.join("beta"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unreadable_dir_reports_error() {
    let b = FileExplorer::new(PathBuf::from("/definitely/not/a/real/dir"));
    assert!(b.error.is_some());
    assert!(b.entries.is_empty());
}

#[test]
fn targets_follow_marks_in_listing_order() {
    let dir = fixture("targets");
    let mut b = FileExplorer::new(dir.clone());
    assert_eq!(b.targets(), [dir.join("Alpha")]);
    b.toggle_mark(3);
    b.toggle_mark(1);
    assert_eq!(b.targets(), [dir.join("beta"), dir.join("zeta.txt")]);
    std::fs::remove_dir_all(dir).unwrap();
}
