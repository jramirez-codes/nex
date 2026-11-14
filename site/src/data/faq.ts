/**
 * Questions people search for. Rendered on the page and, from the same data, as FAQPage
 * structured data (see entry-server.tsx), so the two can never drift apart.
 * Answers are plain text so they work in both places. Keep them true to README.md.
 */
export const FAQ: { q: string; a: string }[] = [
  {
    q: 'What is nex?',
    a: 'nex is a free, open-source, GPU-accelerated terminal emulator for Linux, written in Rust. It has tabs, split panes and a file explorer built in, so you get a tmux-style layout and a file manager in one native-feeling window.',
  },
  {
    q: 'Is nex a tmux alternative?',
    a: 'For layout, yes. nex splits panes right or down, zooms, resizes and moves focus between splits with the keyboard, and every pane has its own header showing the running program and folder. It does not detach sessions or run over SSH like tmux, so keep tmux for that.',
  },
  {
    q: 'Does nex work on Wayland and X11?',
    a: 'Yes. nex runs on Linux under both Wayland and X11, including tiling compositors like Hyprland and Sway. Prebuilt releases are available for x86_64 and aarch64 as .deb packages and tarballs.',
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
    q: 'Is nex available on macOS or Windows?',
    a: 'Not yet. nex is Linux-only today. macOS support is on the roadmap, along with a drop-down quick terminal, a command palette and shell integration.',
  },
  {
    q: 'Is nex free?',
    a: 'Yes. nex is free and open source under the MIT license. The source code, releases and issue tracker are on GitHub.',
  },
]
