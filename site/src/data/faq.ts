/**
 * Questions people search for. Rendered on the page and, from the same data, as FAQPage
 * structured data (see entry-server.tsx), so the two can never drift apart.
 * Answers are plain text so they work in both places. Keep them true to README.md.
 */
export const FAQ: { q: string; a: string }[] = [
  {
    q: 'What is nex?',
    a: 'nex is a free, open-source, GPU-accelerated terminal emulator for Linux and macOS, written in Rust, with an Android app and an iPhone beta. Tabs, split panes, a file explorer and a git Changes pane are built in, so you get a tmux-style layout, a file manager and a diff viewer in one native-feeling window. ssh sessions come along too: splits stay on the server, and files you drop or paste are uploaded. And an Agents pane keeps every Claude Code and Codex agent you run in view, saying which one needs you.',
  },
  {
    q: 'Which coding agents does nex work with?',
    a: 'Claude Code and Codex. nex spots them in any pane, local or over ssh, and shows each one as a card in the Agents pane (Ctrl+Shift+A, Cmd+Shift+A on macOS) with its task, folder and tab, sorted as needs you, done, working or idle. Enter or a click jumps to its pane.',
  },
  {
    q: 'What is an agent pipeline?',
    a: 'A list of stages in .nex/pipelines/NAME.toml, each an agent given a task, whose report feeds the stages after it. Stages can run side by side in git worktrees of their own, send the run back until a reviewer approves, or line up behind one merger that takes their pull requests first come, first served. nex-host pipe NAME "task" runs it, and the Agents pane shows the run as one team card with its stages.',
  },
  {
    q: 'Do agents keep running if I close the pane or lose ssh?',
    a: 'On an ssh host, yes. nex installs a small helper, nex-host, in ~/.nex on the server (no root needed), and agents started through it keep running when you close their pane, lose the connection or quit nex. Their cards stay in the Agents pane marked Detached; click one to open a tab attached to it, with the screen where it left off.',
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
    a: 'For layout, yes. nex splits panes right or down, zooms, resizes and moves focus between splits with the keyboard, and every pane has its own header showing the running program and folder. Splits of a pane running ssh open on the same server, in the same folder, so you do not need tmux on the remote side just to get a second shell there. Sessions survive too: on an ssh host, agents started through nex-host keep running when you close the pane or lose the connection, and with nex --remote (or the phone apps) every pane is a nex-host session that reconnects with its screen intact. A plain ssh pane on a local nex still ends with its connection, so keep tmux there if that matters.',
  },
  {
    q: 'How does nex work with SSH?',
    a: 'Run plain ssh in a pane. Splitting it, or opening a new tab from it, opens another session on the same host in the same remote folder. Dropping files onto it, or pasting a copied image or files, uploads them to a new temp folder on the host and types the remote paths, so a remote program such as Claude Code can open them. nex reuses the pane\'s own ssh options and needs only sh and tar for this (see below for the helper it installs for agents). Turn either part off with ssh-upload = false or ssh-follow = false.',
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
    a: 'Not yet. nex runs on Linux (Wayland and X11) and macOS, and on phones: an Android app ships with every release, and an iPhone app is in beta (build it yourself from nex-ios). Next on the roadmap: custom keybindings, shell integration, themes, a command palette, color emoji and a signed, notarized Mac app.',
  },
  {
    q: 'Is there a phone app?',
    a: 'Yes. Every release includes nex-<version>-android-arm64.apk: install it on the phone, or with adb install -r. The phone runs only nex\'s window and every pane works on your ssh host. You type your password once; the app then adds a key for the phone that expires after 30 days. Panes survive network drops, the keyboard button doubles as an arrow-key joystick and a Ctrl key (double-tap), swipe typing works, holding the button talks to Claude Code\'s voice mode, and the Agents pane lists the agents on the server. An iPhone app is in beta with no download yet: build it with nex-ios/build-ipa.sh on a Mac with Xcode 15 or newer and an Apple Developer account.',
  },
  {
    q: 'Does nex install anything on my servers?',
    a: 'Only a small helper, and you can turn it off. With ssh-agents = true (the default), nex installs nex-host, one static binary, in ~/.nex on hosts its panes connect to, with no root needed, so agents can keep running after their pane closes. rm -rf ~/.nex removes it. With ssh-agents = false nothing is installed, and splits that follow ssh, uploads and pastes still work with plain sh and tar on the server. nex also offers, once per host, to add a few marked lines to your shell config so claude and codex start through the helper; that is up to you.',
  },
  {
    q: 'What are nex-host pipelines?',
    a: 'nex-host is the helper that runs agents on a host. A pipeline is a TOML file in .nex/pipelines/ listing stages, each an agent with a task whose done report feeds the stages after it. nex-host pipe NAME "task" runs it. Stages can run in parallel in their own git worktrees, retry until a reviewer approves, or queue behind one merger that takes pull requests first come, first served. Agents can also message each other directly with nex-host spawn, send, read, done and ls. Two examples ship with it: feature (build, open a PR, review, merge) and pr-team (three builders, one merger).',
  },
  {
    q: 'Is nex free?',
    a: 'Yes. nex is free and open source under the MIT license. The source code, releases and issue tracker are on GitHub.',
  },
]
