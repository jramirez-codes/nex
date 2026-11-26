//! Explorer tests: the shared folder fixture and unit tests of the pane state (here), and UI
//! tests that drive a pane through egui (`ui.rs`).

mod ui;

use super::*;
use std::time::Duration;

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

/// Wait for the listing being read in the background, as a paint would take it.
fn settle(b: &mut FileExplorer) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while b.listing.running() {
        assert!(Instant::now() < deadline, "the listing never arrived");
        std::thread::sleep(Duration::from_millis(1));
        b.poll_listing();
    }
}

/// A stale listing on this machine is re-read on another thread: `refresh_if_stale` returns
/// with the listing, selection, marks and scroll position as they were (and no "Loading…"),
/// and the new listing replaces it once it is in, the same items still selected.
#[test]
fn stale_listing_is_reread_in_the_background() {
    let dir = fixture("background");
    let mut b = FileExplorer::new(dir.clone());
    b.selected = 3;
    b.toggle_mark(1);
    b.selected = 3;
    b.scroll = 17.0;
    std::fs::write(dir.join("Banana.md"), "b").unwrap();
    std::fs::remove_file(dir.join("Apple.md")).unwrap();

    files_changed();
    b.refresh_if_stale();
    assert!(b.listing.running(), "nothing is being read");
    assert!(!b.loading());
    assert_eq!(names(&b), ["Alpha", "beta", "Apple.md", "zeta.txt"]);
    assert_eq!((b.selected, b.scroll), (3, 17.0));

    settle(&mut b);
    assert_eq!(names(&b), ["Alpha", "beta", "Banana.md", "zeta.txt"]);
    assert_eq!(b.entries[b.selected].name, "zeta.txt");
    assert_eq!(b.marked, BTreeSet::from([dir.join("beta")]));
    assert_eq!(b.scroll, 17.0);
    assert_eq!(b.error, None);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A listing not read for a while is re-read the same way.
#[test]
fn old_listing_is_reread_in_the_background() {
    let dir = fixture("old");
    let mut b = FileExplorer::new(dir.clone());
    std::fs::write(dir.join("Banana.md"), "b").unwrap();
    b.last_refresh = Instant::now() - b.backend.refresh_interval();
    b.refresh_if_stale();
    assert!(b.listing.running(), "nothing is being read");
    assert_eq!(names(&b), ["Alpha", "beta", "Apple.md", "zeta.txt"]);
    settle(&mut b);
    assert_eq!(names(&b), ["Alpha", "beta", "Apple.md", "Banana.md", "zeta.txt"]);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Opening another folder forgets a background read of the one left: its answer never
/// replaces the new folder's listing.
#[test]
fn navigating_forgets_the_read_on_its_way() {
    let dir = fixture("forget");
    std::fs::write(dir.join("beta/inside.txt"), "x").unwrap();
    let mut b = FileExplorer::new(dir.clone());
    files_changed();
    b.refresh_if_stale();
    assert!(b.listing.running());
    b.navigate(dir.join("beta"));
    assert!(!b.listing.running());
    std::thread::sleep(Duration::from_millis(20));
    b.poll_listing();
    assert_eq!(names(&b), ["inside.txt"]);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A folder that goes away while shown says so once the background read finds out.
#[test]
fn background_read_reports_a_missing_folder() {
    let dir = fixture("gone");
    let mut b = FileExplorer::new(dir.clone());
    std::fs::remove_dir_all(&dir).unwrap();
    files_changed();
    b.refresh_if_stale();
    settle(&mut b);
    assert!(b.error.is_some());
    assert!(b.entries.is_empty());
}

/// Whether the delete dialog warns about folders is worked out once, when it is asked for,
/// from the listing: the dialog doesn't look at the disk on every frame it is open.
#[test]
fn delete_dialog_knows_about_folders_from_the_listing() {
    let dir = fixture("delete-folder");
    let mut b = FileExplorer::new(dir.clone());
    b.selected = 3;
    b.request_delete();
    assert_eq!(b.confirm_delete.as_deref(), Some(&[dir.join("zeta.txt")][..]));
    assert!(!b.delete_has_folder);
    b.cancel_delete();

    b.toggle_mark(3);
    b.toggle_mark(0);
    b.request_delete();
    assert!(b.delete_has_folder);
    // The answer was taken when asked: the folder vanishing from the disk doesn't change it.
    std::fs::remove_dir_all(dir.join("Alpha")).unwrap();
    assert!(b.delete_has_folder);
    b.cancel_delete();

    // A link to a folder is deleted as a link, without what's inside the folder.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("beta"), dir.join("link")).unwrap();
        b.clear_selection();
        b.refresh();
        let link = b.entries.iter().position(|e| e.name == "link").unwrap();
        assert!(b.entries[link].is_dir && b.entries[link].is_symlink);
        b.selected = link;
        b.request_delete();
        assert!(!b.delete_has_folder);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
