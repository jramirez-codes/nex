# Architecture

nex is a single Rust crate (`src/`, a library with the `nex` program on top) plus `nex-host`
(`nex-host/`), a small helper nex installs on ssh hosts, and `nex-android/` and `nex-ios/`, which
wrap the same library as the Android and iPhone apps. The terminal engine is [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal);
everything on screen is drawn with [egui](https://github.com/emilk/egui) on eframe, which nex
runs on its own winit event loop (`src/platform/winit_app.rs`).

Every module root starts with a `//!` doc saying what it does and how its files divide the work.
Read those first; this page is the map.

## Source tree

```
src/
├── main.rs           the `nex` program: hands over to lib.rs
├── lib.rs            command line, window setup (eframe / winit), the Android entry point
├── cli.rs            command-line flags
├── config.rs         config parsing and reloading
├── layout.rs         binary split tree + directional focus (PaneId, Axis)
├── session/          the saved window layout (session.json)
│   ├── mod.rs            its format, saving and loading, the single-writer lock
│   └── claude.rs         restored Claude Code panes resume their conversation
├── app/              the application: state and frame loop (mod.rs), plus
│   ├── actions.rs        the actions and carrying them out
│   ├── reload.rs         live config reload
│   ├── restore.rs        saving the window into a session and restoring one at launch
│   ├── connect.rs        the connect screen, where panes can't run local shells (Android)
│   ├── input/            keyboard, keymap (shortcut tables per OS), mouse, zoom
│   ├── chrome/           title bar and tab strip, mobile sidebar, window edges and drops
│   ├── workspace/        tabs, splits, opening and closing panes, drawing panes, pane drag,
│   │                     the sticky pane, and keeping Agents panes up to date
│   ├── animation/        pane motion, tab close, tab-switch transitions, sticky glide
│   ├── ssh/              following ssh sessions (follow), uploads, the mic button, the hosts
│   │                     whose agents the Agents pane shows (hosts), connection sharing (sharing)
│   └── tests/            whole-App tests driven through egui
├── terminal/         one terminal per pane
│   ├── mod.rs            PTY + VT state, title and cwd
│   ├── input.rs          keyboard & mouse → xterm escape sequences
│   ├── osc7.rs           PTY wrapper that picks up OSC 7 (the shell's working directory)
│   ├── remote.rs         a pane whose shell is a nex-host session on a host nex connects
│   │                     to itself (the Android app, `--remote`)
│   ├── search.rs         scrollback search, URL and OSC 8 link detection
│   └── snapshot.rs       copy of the visible screen, taken under the lock
├── render/           drawing the terminal grid
│   ├── mod.rs            runs, colors, cursor, selection, search highlights
│   ├── glyphs.rs         glyph cache + per-pane mesh
│   ├── boxdraw.rs        procedural box / block / braille / Powerline glyphs
│   └── fonts.rs          font setup + embedded Nerd Font
├── file_explorer/    the explorer pane
│   ├── mod.rs            state, navigation, selection
│   ├── fs/               listing, copying, moving, path quoting; file clipboard and jobs;
│   │                     remote.rs: the same on an ssh host, as shell scripts run there
│   ├── input/            clicks, touch gestures, drags, keys; renaming, location bar, zoom
│   ├── view/             path bar, listing rows and tiles, action bar, Properties dialog
│   └── tests/            the folder fixture, unit tests and egui-driven UI tests
├── git_diff/         the Changes pane (Ctrl+G): read-only git diff viewer
│   ├── mod.rs            state, background loading, keys
│   ├── git.rs            running git (here, or on an ssh host), parsing status and diffs
│   ├── syntax.rs         syntax colors (syntect with bat's grammars)
│   └── view/             drawing and the mouse: the layout, toolbar, file list and diff
├── agents/           the Agents pane: every Claude Code / Codex agent at a glance
│   ├── mod.rs            the pane's state and keys
│   ├── model.rs          which panes are agents and what they're doing (Tracker)
│   ├── motion.rs         cards moving between sections
│   └── view.rs           drawing and the mouse
├── remote/           ssh: nex's own connections, uploads, mic sharing (+ audio capture),
│                     installing and watching nex-host; ssh/ is nex's own ssh client (russh):
│                     host keys, the one-time password and device key, reconnecting;
│                     exec.rs runs shell scripts on a pane's host (for Files and Changes
│                     panes opened from an ssh pane), over `ssh` or nex's own connection
├── ui/               shared UI kit: theme, glide/ink animation, painting, icons, widgets, toasts
└── platform/         everything OS-specific, behind one API (see platform/mod.rs)
    ├── mod.rs            the shared types and the per-OS switch
    ├── unix.rs           POSIX pieces shared by Linux and macOS
    ├── winit_app.rs      nex's own winit event loop driving eframe
    ├── fontdb.rs         font lookup without fontconfig (macOS, Windows)
    ├── no_dnd.rs         drag and drop stub where winit handles it
    ├── no_watch.rs       folder-watch stub where there is no inotify
    ├── linux/            /proc process info, fontconfig, Wayland drag and drop, inotify,
    │                     clipboard, Open With, the FileManager1 D-Bus service
    ├── macos/            libproc process info, traffic lights, app menu, clipboard,
    │                     Open With, starting from nex.app
    ├── windows/          compiles, not supported yet
    ├── android/          app folders, stdout/stderr to logcat, no local shells (a thin
    │                     client: every pane works on an ssh host)
    └── ios/              the same thin client: full-screen window with the safe area and
                          keyboard as the content rect, UIPasteboard, haptics

nex-android/          the Android app: nex's library as NativeActivity's shared library
                      (see nex-android/README.md)
nex-ios/              the iPhone app: nex's library as a static library, linked and signed by
                      a small Xcode project (see nex-ios/README.md)
nex-host/             the helper on ssh hosts (see nex-host/README.md); nex itself links only
                      its protocol (`proto`, `claude`, `paths`), built without the `daemon` feature
```

Outside the Rust code:

| Path | What it is |
|---|---|
| `assets/` | fonts, icons, `.desktop` files and the D-Bus service file |
| `packaging/macos/` | `nex.app`'s `Info.plist`, icon and `make-app.sh` (used by `install.sh` and the release workflow) |
| `site/` | the website (Vite + React, prerendered), deployed by `.github/workflows/pages.yml` |
| `bench/` | latency and memory benchmarks against other terminals (see `bench/README.md`) |
| `tools/demo/` | scripts that record the demo GIFs |

## Rules the code follows

- **Nothing outside `platform/` checks the OS.** Porting nex to another OS means adding a folder
  there that provides the same items (`platform/mod.rs` lists them).
- **Panes live in the split tree** (`layout.rs`), keyed by `PaneId`. A pane is a terminal, a file
  explorer, a Changes pane or an Agents pane (`layout::PaneKind`); `App` keeps each kind's state
  in its own map. Code that treats kinds differently matches on `App::pane_kind(id)` rather
  than probing the maps, so the compiler finds every place a new kind needs handling.
- **`ui/` is stateless and shared.** Colors shared across the chrome belong in `ui/theme.rs`;
  easing curves, `smoothstep` and `Lerp` in `ui/glide.rs`; font size limits in `config.rs`.
- **Background work never blocks a frame.** Git, file copies, ssh jobs and process polls run on
  threads and hand results back; the frame loop picks them up (`App::poll_background`).

## Where to start for common changes

- **A new action or shortcut:** `app/actions.rs` (the action), `app/input/keymap.rs` (the key).
- **A new pane kind:** add it to `layout::PaneKind` and give it a map on `App` (`app/mod.rs`),
  then follow the compiler through every `match` on `App::pane_kind` (drawing, labels, keys,
  zoom, the title-bar deck). Beyond those: `pane_kind` and `forget_pane`
  (`app/workspace/mod.rs`), opening it (`app/workspace/`), and reading it back from a session
  (`session/mod.rs`, `app/restore.rs`). The Changes pane (`git_diff/`) is a compact example.
- **Something on an ssh host:** the work goes in `remote/` (no UI), the buttons, dialogs and
  toasts in `app/ssh/`.
- **A config option:** `config.rs`, then document it in `config.example` and the README.

## Demo recordings

The GIFs in `site/public/demo/` are recordings of the real app: `tools/demo/record.py` plays the
scenes in `tools/demo/scenes.py` (Hyprland), `tools/demo/make_gifs.py` cuts them into clips, and
`tools/demo/promo.py` puts the promo clip on a backdrop (`--no-captions` makes the README's `hero.gif`).
