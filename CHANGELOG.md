# Changelog

All notable changes to nex are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and nex uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A Files tab: the directory label in the title bar is now a tab that browses the current
  folder (click again to go back, middle-click to close). Folders open in place, files open
  in their default app, and the pane splits like any other, starting new terminals in the
  folder you're browsing.

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

[Unreleased]: https://github.com/jramirez-codes/nex/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/jramirez-codes/nex/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jramirez-codes/nex/releases/tag/v0.1.0
