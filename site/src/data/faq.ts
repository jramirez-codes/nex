/**
 * Questions people search for. Rendered on the page and, from the same data, as FAQPage
 * structured data (see entry-server.tsx), so the two can never drift apart.
 * Answers are plain text so they work in both places. Keep them true to README.md.
 */
export const FAQ: { q: string; a: string }[] = [
  {
    q: 'What is nex?',
    a: 'nex is a free, open-source, GPU-accelerated terminal emulator for Linux and macOS, written in Rust. It has tabs, split panes and a file explorer built in, so you get a tmux-style layout and a file manager in one native-feeling window. ssh sessions come along too: splits stay on the server, and files you drop or paste are uploaded.',
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
    a: 'For layout, yes. nex splits panes right or down, zooms, resizes and moves focus between splits with the keyboard, and every pane has its own header showing the running program and folder. Splits of a pane running ssh open on the same server, in the same folder, so you do not need tmux on the remote side just to get a second shell there. nex does not detach sessions, so keep tmux if work has to survive a dropped connection.',
  },
  {
    q: 'How does nex work with SSH?',
    a: 'Run plain ssh in a pane. Splitting it, or opening a new tab from it, opens another session on the same host in the same remote folder. Dropping files onto it, or pasting a copied image or files, uploads them to a new temp folder on the host and types the remote paths, so a remote program such as Claude Code can open them. nex reuses the pane\'s own ssh options and needs nothing installed on the server. Turn either part off with ssh-upload = false or ssh-follow = false.',
  },
  {
    q: 'Why does nex offer to edit ~/.ssh/config?',
    a: 'Only for hosts that ask for a password, and only if you agree. Uploads and new sessions cannot type your password, so nex offers connection sharing: a Match block at the end of ~/.ssh/config that applies only to ssh started inside nex, letting them reuse the login you typed in the pane. It is marked "Added by nex" and safe to delete. Hosts you reach with keys or an agent need nothing.',
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
    a: 'Not yet. nex runs on Linux (Wayland and X11) and macOS. Next on the roadmap: custom keybindings, a command palette, themes and a signed, notarized Mac app.',
  },
  {
    q: 'Is nex free?',
    a: 'Yes. nex is free and open source under the MIT license. The source code, releases and issue tracker are on GitHub.',
  },
]
