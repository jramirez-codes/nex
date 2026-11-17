# Changelog

All notable changes to nex are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and nex uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Files and images reach SSH sessions. Dropping files onto a pane running `ssh`, or pasting
  a copied image or copied files there (Ctrl+V or Ctrl+Shift+V; Cmd+V on macOS), copies
  them to a new folder under the remote host's `$TMPDIR` (or `/tmp`) and types the remote
  paths, so a program on the other end, such as Claude Code, can open them. The upload runs
  over a second `ssh` with the pane's own options and `BatchMode`, so it can't type a
  password. When a host asks for one, nex offers to turn on connection sharing: a short block
  at the end of `~/.ssh/config`, matching only ssh run inside nex, that makes each session a
  `ControlMaster` the upload can ride on. Reconnect once and later uploads reuse the login
  typed into the pane. Hosts reached with keys or an agent need nothing. If an upload can't
  log in, the local paths are typed instead. Turn it off with `ssh-upload = false`.
- Splits and new tabs follow SSH sessions. Splitting a pane running `ssh`, or opening a new
  tab from it, opens another session on the same host, in the same remote folder. Typing
  `exit` there drops to a local shell, as in the original pane. With connection sharing on,
  the new session reuses the pane's login. When it's off and the host would ask for a password
  again, nex offers to turn it on first. The remote folder comes from the remote shell's
  OSC 7 report (fish sends one; see the README for bash and zsh), or from a `user@host: dir`
  window title such as Debian and Ubuntu's bash sets. Otherwise the session opens in the
  remote home. Turn it off with `ssh-follow = false`.
- nex can be your file manager. `nex --explorer` opens on the Files tab, and a folder argument
  (`nex ~/Downloads`, or a `file://` URI) opens the Files tab in that folder. A new hidden
  `nex-files.desktop` entry handles `inode/directory`, so
  `xdg-mime default nex-files.desktop inode/directory` makes folders open in nex.
- `explorer-view = list | grid` picks the layout file explorers open in.

### Fixed

- Ctrl+V with an image (and no text) on the clipboard now reaches the program as `^V`, so
  Claude Code can paste the image itself. Before, nex swallowed the key.
- An image dragged in from a browser (Firefox, Chromium) onto a pane now drops. nex saves it
  to a temp file and types that path, so Claude Code picks it up as an image. Dropped on an
  explorer, it lands in that folder. Before, the drop did nothing on Wayland: the drag's only
  URI was the image's web address, not a file.
- nex could crash as it quit on Wayland: the drag and drop thread kept reading the
  display's connection while it was being closed. The thread now stops first.

## [0.9.1] - 2026-11-16

### Changed

- The glide tab switch now moves the panes the same way the glass pill moves, so the old tab
  glides out ahead of the pill and the new one follows it in, instead of going against it.

## [0.9.0] - 2026-11-16

### Added

- Switching tabs animates. By default the old tab glides out toward the side it is leaving
  as the new one glides in. `tab-animation = ink` makes the new tab soak through the old one
  like ink instead, from a drop under the tab you picked, and `tab-animation = none` switches
  instantly. `animations = false` still turns every animation off.
- Closing a tab animates. The active tab's glass pill folds into its × and grows back out
  onto the tab now showing; a background tab shrinks into its × and fades. The tabs beside
  it slide over to fill the space.
- Zooming a pane scales its card and text between its slot and the whole tab, with the other
  panes fading out and back in under it. Equalizing splits eases the panes into place.

## [0.8.2] - 2026-11-16

### Changed

- nex uses much less memory at idle. On macOS its resident memory drops from about 480 MB to
  about 60 MB: font lookup now maps font files instead of reading every system font into
  memory.
- egui's glyph atlas is capped at 4096 px wide instead of the GPU's maximum (16384 px), which
  saves a few MB in memory and on the GPU.
- Memory freed during startup and after a font change is given back to the system (on Linux,
  glibc also keeps fewer per-thread heaps).

### Added

- `bench/mem.sh` shows where nex's idle memory goes on Linux: heap, GL driver, the binary
  and fonts.

## [0.8.1] - 2026-11-16

### Fixed

- On macOS, the window buttons (traffic lights) no longer flicker up for a frame when the
  terminal's title changes.
- On macOS, tab tiles and pane header icons follow the program running in the pane again
  (Claude Code, yazi, and the rest). Shells start through `login`, which runs as root, and
  nex couldn't look past it, so every pane kept its shell icon.
- On macOS, Claude Code's native install shows as `claude` rather than its version number.

### Changed

- Refreshed the benchmark results in the README and on the site.

## [0.8.0] - 2026-11-15

### Added

- Swap panes. Drag a pane by its header onto another pane: the card follows the mouse or
  finger, the pane under it slides into the free slot as a preview, and letting go swaps
  them (`Esc`, or letting go elsewhere, puts it back). `Alt+Shift+Arrows`
  (`Cmd+Shift+Option+Arrows` on macOS) swaps the focused pane with its neighbor.
- File explorer, Thunar-style:
  - Back / Forward (`Alt+Left` / `Alt+Right`, the mouse's side buttons), and clickable
    breadcrumbs that are also drop targets. `Ctrl+L`, or a click beside the breadcrumbs, types
    a location, with `Tab` completion.
  - Rename in place (`F2`), new folders (`Ctrl+N`, `Cmd+Shift+N` on macOS) and new empty files.
  - Copies, moves and deletes show a live progress bar in the action bar: bytes done, percent,
    time left and the current file, with a Cancel button.
  - Open With (from the desktop's app list on Linux, installed apps on macOS), Open Terminal
    Here, and a Properties dialog (`Alt+Enter`, `Cmd+I` on macOS) with kind, size or folder
    contents, dates, permissions and owner, where the name can be edited too.
  - Type to jump to a name, and icons by file type (code, images, video, audio, archives,
    documents, executables and more), with marks on symlinks and well-known folders.
  - Dragging files carries the grabbed item's own icon, at the size it has in the listing and
    held where it was grabbed (with a count when several go). On Wayland the icon keeps
    following the pointer after the drag leaves the window, instead of disappearing.

### Changed

- Faster launch. The first shell starts before the window opens, so its startup overlaps
  window and GPU setup, and the system font lookups run in parallel instead of one after
  another. On one Linux machine, the first frame arrives in about 123 ms (was 216 ms).
- On Wayland, dragging files from the explorer into another window (another nex, or a file
  manager such as Thunar or Nautilus) now moves them, as a drag within the window does. Before,
  it always copied and left the original in place. Files dropped into a nex explorer from
  another window move too; hold `Ctrl` to copy instead.
- File explorers watch their folder (inotify on Linux), so a file moved away by another
  window, `mv` in a terminal or a file manager disappears at once, even in a window in the
  background. Elsewhere listings are still re-read every couple of seconds.

### Fixed

- Drag and drop between windows on Hyprland. Hyprland only sends drags and the clipboard to
  the first data device a client makes, which was egui's clipboard, so no drag ever reached
  nex and drags between two nex windows were cancelled. nex now runs eframe on its own event
  loop, makes its data device before the window opens, and reads pasted text from it.
- "Application not responding" on Wayland for nex windows on another workspace. With vsync
  on, drawing a frame waits for the compositor's frame callback, which a hidden window never
  gets, so nex's main thread blocked and missed the compositor's pings. On Wayland vsync is
  now off and nex paces frames to the display's refresh rate itself.

## [0.7.4] - 2026-11-14

### Fixed

- macOS: in full screen, the title bar kept an empty gap where the hidden window buttons
  would be. The workspace label and tabs now start at the left edge.

## [0.7.3] - 2026-11-14

### Fixed

- macOS: the Dock icon switched from the nex collie to the egui logo once nex had started.
  nex now keeps the app's own icon.
- macOS: the nex icon had a white square behind its rounded corners. `nex.icns` is
  re-rendered with a transparent background by the new `packaging/macos/make-icns.sh`.

## [0.7.2] - 2026-11-14

### Fixed

- macOS: the close / minimize / zoom buttons sat above the middle of nex's title bar. They
  are now centered in it, in line with the tabs.

## [0.7.1] - 2026-11-14

### Added

- `nex.app` for macOS, with the nex icon and the name "nex" in the menu bar. Releases include
  it as `nex-<version>-macos-app.zip`, and `install.sh` installs it to `/Applications` (or
  `~/Applications`). Built by `packaging/macos/make-app.sh`; ad-hoc signed, not notarized.

### Fixed

- macOS: opened from Finder or the Dock, the first shell started in `/` instead of your home
  folder, and without `LANG`, so programs fell back to ASCII. nex now starts it at home and
  sets `LANG` from your region (e.g. `en_US.UTF-8`) when it's missing.

## [0.7.0] - 2026-11-14

### Added

- macOS support. Releases include a universal binary (Apple Silicon and Intel) as a tarball
  with `install.sh`; it isn't notarized yet. Shortcuts follow Ghostty's macOS defaults (Cmd+T, Cmd+D,
  Cmd+C / Cmd+V, Cmd+1…9; Ctrl+C always reaches the shell), the window uses the native traffic
  lights, pane headers show the foreground process and folder, and fonts are found without
  fontconfig. Dragging files out to Finder isn't supported yet.

### Changed

- Everything OS-specific now lives in `src/platform/`, one folder per OS behind the same API
  (Wayland drag and drop moved there from `src/wl_dnd/`). The keyboard shortcuts are in
  `src/app/keymap.rs`, with one table per OS. Pane-header tooltips show the shortcuts for the
  current OS.
- `config.example` no longer sets `window-decorations` / `window-controls`, since their
  defaults now depend on the OS.
- `install.sh` works on macOS (binary and config only; the desktop entry and icon are
  Linux-only).

## [0.6.3] - 2026-11-14

### Fixed

- Launching nex from an app launcher (rofi, Hyprland, etc.) did nothing when the session's PATH
  didn't include `~/.local/bin`. `install.sh` now writes the absolute binary path into the
  installed desktop entry's `Exec=` line.

## [0.6.2] - 2026-11-14

### Fixed

- Symbols such as ✢ ✻ ✘ ☒ ⎯ ⧉ (used by Claude Code's spinner, todos and rules) drew as empty
  boxes. The symbol fallback font was looked up as "Noto Sans Symbols 2" instead of
  fontconfig's "Noto Sans Symbols2", so it never loaded. nex now also falls back to DejaVu
  Sans Mono, Noto Sans Symbols and Noto Sans Math when they're installed.

## [0.6.1] - 2026-11-14

### Changed

- New logo and app icon: a border collie head in the accent gradient, replacing the prompt
  icon. It's used for the desktop icon, the README and the website.

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

[Unreleased]: https://github.com/jramirez-codes/nex/compare/v0.9.1...HEAD
[0.9.1]: https://github.com/jramirez-codes/nex/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/jramirez-codes/nex/compare/v0.8.2...v0.9.0
[0.8.2]: https://github.com/jramirez-codes/nex/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/jramirez-codes/nex/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/jramirez-codes/nex/compare/v0.7.4...v0.8.0
[0.7.4]: https://github.com/jramirez-codes/nex/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/jramirez-codes/nex/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/jramirez-codes/nex/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/jramirez-codes/nex/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/jramirez-codes/nex/compare/v0.6.3...v0.7.0
[0.6.3]: https://github.com/jramirez-codes/nex/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/jramirez-codes/nex/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/jramirez-codes/nex/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/jramirez-codes/nex/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/jramirez-codes/nex/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/jramirez-codes/nex/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/jramirez-codes/nex/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jramirez-codes/nex/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/jramirez-codes/nex/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/jramirez-codes/nex/releases/tag/v0.1.0
