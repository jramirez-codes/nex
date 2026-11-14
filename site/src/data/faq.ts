/**
 * Questions people search for. Rendered on the page and, from the same data, as FAQPage
 * structured data (see entry-server.tsx), so the two can never drift apart.
 * Answers are plain text so they work in both places. Keep them true to README.md.
 */
export const FAQ: { q: string; a: string }[] = [
  {
    q: 'What is nex?',
    a: 'nex is a free, open-source, GPU-accelerated terminal emulator for Linux and macOS, written in Rust. It has tabs, split panes and a file explorer built in, so you get a tmux-style layout and a file manager in one native-feeling window.',
  },
  {
    q: 'Does nex run on macOS?',
    a: 'Yes. Releases include nex.app, a universal build that runs natively on Apple Silicon and Intel Macs. It uses the native traffic-light window buttons and Cmd-based shortcuts that follow Ghostty (Cmd+T, Cmd+D, Cmd+E), while Ctrl+C and other Ctrl keys always reach the shell and Option types accented characters as usual.',
  },
  {
    q: 'How do I install nex on a Mac?',
    a: 'Download nex-<version>-macos-app.zip from the latest GitHub release, unzip it and drag nex.app to Applications. nex is not notarized yet, so run "xattr -dr com.apple.quarantine /Applications/nex.app" once before opening it. There is also a tarball that installs nex.app plus the nex command, or you can build from source with the Xcode command line tools.',
  },
  {
    q: 'Is nex a tmux alternative?',
    a: 'For layout, yes. nex splits panes right or down, zooms, resizes and moves focus between splits with the keyboard, and every pane has its own header showing the running program and folder. It does not detach sessions or run over SSH like tmux, so keep tmux for that.',
  },
  {
    q: 'Does nex work on Wayland and X11?',
    a: 'Yes. nex runs on Linux under both Wayland and X11, including tiling compositors like Hyprland and Sway. Prebuilt Linux releases are available for x86_64 and aarch64 as .deb packages and tarballs.',
  },
  {
    q: 'How does nex compare to Alacritty, kitty and Ghostty?',
    a: 'nex is built on alacritty_terminal, the same VT engine behind Alacritty and Zed. Like kitty and Ghostty it has tabs and splits, and on top of that it adds a built-in file explorer and per-pane headers. The Performance section above compares throughput, startup time and memory on the same machine.',
  },
  {
    q: 'Do I need to install a Nerd Font?',
    a: 'No. JetBrains Mono Nerd Font is embedded, so icons in Neovim, starship, lazygit and other TUIs work out of the box. If you have a Nerd Font Mono installed, nex uses it instead.',
  },
  {
    q: 'How do I configure nex?',
    a: 'Edit ~/.config/nex/config, a Ghostty-style key = value file. Fonts, colors, opacity, cursor style, the shell command and more apply as soon as you save, with no restart.',
  },
  {
    q: 'Is nex available on Windows?',
    a: 'Not yet. nex runs on Linux (Wayland and X11) and macOS. Next on the roadmap: a drop-down quick terminal, a command palette and shell integration.',
  },
  {
    q: 'Is nex free?',
    a: 'Yes. nex is free and open source under the MIT license. The source code, releases and issue tracker are on GitHub.',
  },
]
