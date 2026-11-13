# Changelog

All notable changes to nex are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and nex uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/jramirez-codes/nex/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/jramirez-codes/nex/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/jramirez-codes/nex/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jramirez-codes/nex/releases/tag/v0.1.0
