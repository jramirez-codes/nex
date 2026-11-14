# Changelog

All notable changes to nex are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and nex uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.6.0] - 2026-11-14

### Added

- Clipboard toasts: copying (Ctrl+Shift+C or copy-on-select) or pasting (Ctrl+Shift+V) in a
  terminal shows a short "Copied 42 characters" / "Pasted 3 lines" toast at the bottom of the
  window. `clipboard-toasts = false` turns them off.

### Changed

- nex is now "the nex terminal" (was "the nex-gen terminal") in the README, the site and the
  package description. The site's version tag shows 0.6.

## [0.5.0] - 2026-11-14

### Added

- `mobile = true` for phone-sized touch screens: the title bar becomes a sidebar on the
  left, with the window buttons, the Files tab, one large app icon per tab (title
  underneath) and the new-tab button stacked top to bottom. Split tabs fan their icon deck
  out wider, so the shuffle when focus moves is easy to see. Pane headers get taller, with
  finger-sized buttons.
- Drag and drop between windows on Wayland: drag files out of the explorer into another nex
  window (onto a terminal types their paths, onto an explorer copies them) or into another
  app, and drop files from other apps into nex. winit has no Wayland drag and drop, so nex
  speaks the protocol itself (`src/wl_dnd.rs`). On sway a finger's drag stays with the window
  it started in, so nex passes it to the nex window under the finger over a local socket.
  A window that receives files from another window takes keyboard focus, and the window a
  drag left no longer thinks the finger (or button) is still down. `NEX_DEBUG_DND=1` logs each
  step.

### Fixed

- Files dropped where there's no pane (between panes, or outside the window) are ignored.
  They used to go to the focused pane, so a drop on nothing could copy files into the
  explorer's folder.
- Touchscreen drag and drop onto a terminal now drops where the finger lifts. Before, it
  could use the hidden mouse cursor's position (sway moves the pointer there when a finger
  lifts), so the paths went to the wrong terminal or nowhere.

## [0.4.0] - 2026-11-14

### Added

- Command-line flags: `--config=PATH` reads another config file, `--app-id=ID` sets the
  window's Wayland app_id (X11 class), `--working-directory=DIR` starts the first shell there,
  and `-e COMMAND [ARGS...]` runs a program in the first pane instead of the shell. Also
  `--help` and `--version`. A new window (Ctrl+Shift+N) keeps `--config` and `--app-id`.
- Delete files and folders in the explorer: the `Delete` key, "Delete…" in the right-click
  menu, or the Delete button in the action bar (for touchscreens). A confirmation dialog warns
  that items are permanently deleted (not moved to the trash) before anything is removed.
- Touchscreen drag and drop: while selecting, drag a selected item to move the selection
  without another long press. The drag label floats above your finger, and a terminal under
  the drag lights up with a "Drop to insert path" hint. Dropping anywhere on a terminal's card,
  header included, types the paths.

## [0.3.0] - 2026-11-13

### Added

- Pane animations: a split slides the new pane out of the original's edge, a closed pane
  collapses and fades while its neighbor grows into the space, and focus dimming fades instead
  of switching. Terminals are sized for the final layout up front, so the shell never sees a
  resize mid-animation.
- `animations = false` turns off every UI animation (panes, the title-bar tab pill and the
  pane-icon deck). It applies on live reload.

### Changed

- Faster terminal drawing: each pane is now one mesh of glyph quads taken from a per-character
  glyph cache, instead of text laid out every frame. A full screen of changing colored text
  (htop-style redraws) uses about 2.4x less CPU, with identical output.
- Output in background tabs no longer redraws the window, so many busy tabs cost much less.
- Panes whose contents haven't changed reuse their last frame instead of being rebuilt, so
  one busy pane among many idle splits only costs about what that one pane does.
- Lower memory use: font files are memory-mapped instead of read into memory (egui copied
  every font it was handed, so each one was held twice). Idle memory drops by about 55 MB with
  a system Nerd Font and the CJK fallback installed.

### Fixed

- A key press that runs a shortcut (such as `Alt+1`) no longer also sends a stray character
  to the shell.

## [0.2.0] - 2026-11-13

### Added

- A Files tab: the directory label in the title bar is now a tab that browses the current
  folder (click again to go back, middle-click to close). Folders open in place, files open
  in their default app, and the pane splits like any other, starting new terminals in the
  folder you're browsing.
- `Ctrl+E` splits a file explorer off the focused pane, open at that pane's folder.
- File management in the explorer: list and grid views, multi-select (keyboard, Ctrl/Shift+click,
  long-press), copy, cut and paste, a right-click menu and an action bar, and drag and drop
  (into folders, onto terminals as paths, and from other apps).
- Pinch zoom (touchscreen or touchpad) and Ctrl+wheel, for terminal fonts and explorer
  listings.
- `window-controls = false` hides the close, minimize and maximize buttons in the title bar.

### Changed

- The config reloads automatically when you save the file, not only on `Ctrl+Shift+,`.

### Fixed

- Reloading the config now updates `scrollback-limit`, `cursor-style`, `cursor-style-blink` and
  `window-decorations` in the running window, and keeps per-pane zoom unless `font-size`
  changed.

## [0.1.1] - 2026-11-13

### Changed

- The active tab's glass pill now glides to the newly selected tab (200ms, ease-out) instead
  of jumping, whether you click, switch with the keyboard, close or reorder tabs.

## [0.1.0] - 2026-11-13

The first public release of nex, the nex-gen terminal.

### Added

- GPU-rendered terminal built on `alacritty_terminal` and egui.
- Tabs in the title bar, in a segmented-control track with a glass active pill, per-app icons
  and fading titles. Drag tabs to reorder them.
- Split panes as rounded cards with headers, plus split, zoom and close buttons, draggable
  dividers, directional focus and keyboard resizing.
- Smart titles showing the foreground process and its directory, e.g. `Node ~/code/nex`.
- Truecolor and 256-color support, and bold, italic, underline, strikethrough, dim and inverse.
- Procedurally drawn box-drawing, block, braille and Powerline glyphs, so TUI borders line up
  exactly.
- Embedded JetBrains Mono Nerd Font Mono. An installed Nerd Font Mono is used if present.
- Scrollback search (`Ctrl+Shift+F`) with match highlighting.
- Ctrl+click for URLs and OSC 8 hyperlinks.
- Mouse selection (word, line and block), SGR mouse reporting, alternate scroll and bracketed
  paste.
- Ghostty-style config file at `~/.config/nex/config`, with live reload.
- Release builds for x86_64 and aarch64 Linux, as a tarball and a `.deb`.

[Unreleased]: https://github.com/jramirez-codes/nex/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/jramirez-codes/nex/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/jramirez-codes/nex/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/jramirez-codes/nex/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/jramirez-codes/nex/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jramirez-codes/nex/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/jramirez-codes/nex/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jramirez-codes/nex/releases/tag/v0.1.0
