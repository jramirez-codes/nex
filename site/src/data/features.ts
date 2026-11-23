export interface Feature {
  eyebrow: string
  title: string
  body: string
  keys?: { linux: string[]; mac: string[] }
  shot?: { name: string; alt: string } // public/demo/<name>.mp4 + .jpg poster
}

/** The big alternating feature rows. Copy follows README.md and CHANGELOG.md. */
export const SPOTLIGHT: Feature[] = [
  {
    eyebrow: 'Tabs and splits, built in',
    title: 'Every split is a card.',
    body: 'Tabs live in the title bar on a segmented track with a glass pill that glides to the one you pick, and the panes glide along with it. Close a tab and the pill folds into its × while its neighbors slide over. Split right or down and the new pane slides out of the old one’s edge; close one and its neighbor grows into the space. Zoom a pane and it scales up to fill the tab, or drag the gaps to resize. Each pane has its own header with what is running and where.',
    keys: { linux: ['Ctrl+Shift+O', 'Ctrl+Shift+E', 'Ctrl+Shift+Enter'], mac: ['Cmd+D', 'Cmd+Shift+D', 'Cmd+Shift+Enter'] },
    shot: { name: 'splits', alt: 'Splitting a tab right and down, zooming the nvim pane, then resizing and equalizing the splits' },
  },
  {
    eyebrow: 'Only in nex',
    title: 'Your files and your changes, next to your shell.',
    body: 'One shortcut and a file explorer splits off the current pane, open at its folder. List or grid view, multi-select, copy, cut and paste. Drag files onto a terminal to type their quoted paths, or split the explorer to start a shell in the folder you are looking at. Another opens the Changes pane: what git sees as changed, each diff as hunk cards, following the repo you are in. Both work over ssh too.',
    keys: { linux: ['Ctrl+E', 'Ctrl+G', 'Ctrl+H'], mac: ['Cmd+E', 'Cmd+G', 'Cmd+H'] },
    shot: { name: 'explorer', alt: 'Ctrl+E opens a file explorer beside the shell; three files are selected, grid view toggled, and a shell split off in src' },
  },
  {
    eyebrow: 'Scrollback search and links',
    title: 'Find anything you have scrolled past.',
    body: 'A floating search field highlights every match in the scrollback and steps through them. Hold Ctrl (Cmd on a Mac) to underline URLs and OSC 8 hyperlinks, and click to open them.',
    keys: { linux: ['Ctrl+Shift+F'], mac: ['Cmd+F'] },
    shot: { name: 'search', alt: 'Scrollback search highlighting every match of “pane” and stepping through them' },
  },
]

/** The smaller grid of details. Keep the count a multiple of 4 so the grid's rows stay full. */
export const DETAILS: { icon: string; title: string; body: string }[] = [
  { icon: '🛰', title: 'Every pane on a host', body: 'nex --remote=user@host opens a window whose every pane lives on that host, as a nex-host session that reconnects when the network drops.' },
  { icon: '🗂', title: 'Files and Changes over ssh', body: 'In an ssh pane, Ctrl+E (Cmd+E) and Ctrl+G (Cmd+G) open the explorer and the git Changes pane on the host, in the session’s folder.' },
  { icon: '🎨', title: 'Diffs in your colors', body: 'The Changes pane highlights each diff for its language, in colors taken from your terminal palette, with changed words tinted on top.' },
  { icon: '⟲', title: 'Picks up where you left off', body: 'A plain nex brings back every tab and split, re-runs what each pane was running, resumes Claude Code (claude --resume) and reattaches agents on your servers.' },
  { icon: '●', title: 'Tabs that tap your shoulder', body: 'A bell or a desktop notification (OSC 9, 99 or 777) in a tab you aren’t looking at lights a dot on it, and flags the dock or taskbar while nex is in the background.' },
  { icon: '✥', title: 'Drag panes around', body: 'Drag a pane by its header onto another to swap them, onto an edge to put it there, onto the title bar for a tab of its own, or over another tab to move it.' },
  { icon: '📁', title: 'Your file manager, too', body: 'nex --explorer opens on the Files tab, nex --select=PATH opens a folder with a file selected, and nex can be the default handler for folders.' },
  { icon: '📋', title: 'Paste, confirmed', body: 'Small “Copied …” and “Pasted …” toasts say what moved. An image on the clipboard reaches Claude Code, which pastes it itself.' },
  { icon: '🎙', title: 'Your mic over ssh', body: 'One click on an ssh pane’s header shares your microphone as the host’s default mic, so voice mode works on a server. Experimental.' },
  { icon: '▤', title: 'Tabs in a sidebar', body: 'tab-bar = sidebar moves the tabs to a left sidebar with bigger buttons, made for touch screens.' },
  { icon: '⛓', title: 'Pipelines', body: 'A TOML file of stages, each an agent with a task: plan, build in parallel worktrees, review, retry, merge. nex-host pipe runs it.' },
  { icon: '🌿', title: 'Worktrees, tidied', body: 'nex-host worktree ls lists the git worktrees agents made; worktree prune removes the merged ones nobody is working in, with their branches.' },
  { icon: '📖', title: 'A skill for your agents', body: 'nex-host skill install teaches Claude Code and Codex to use nex-host on their own: spawn helpers, message each other, run pipelines.' },
  { icon: '🧪', title: 'Example pipelines', body: 'nex-host ships ready-made pipelines: a feature build that opens, reviews and merges a PR, and a team of builders feeding one merger.' },
  { icon: '✉', title: 'Agents that talk', body: 'nex-host spawn, send, read, done and ls let agents start helpers, ask each other questions and report back, in messages typed into their input.' },
  { icon: '📌', title: 'Pin to every tab', body: 'Ctrl+Shift+P (Cmd+Shift+P) pins a pane, say the Agents pane, so it follows you from tab to tab and glides along.' },
  { icon: '⚡', title: 'Drawn by the GPU', body: 'Each pane is one mesh of cached glyphs, and idle panes reuse their last frame, so btop or nvim stay smooth with CPU to spare.' },
  { icon: '▦', title: 'Pixel-perfect TUIs', body: 'Box-drawing, block, braille and Powerline glyphs are drawn as geometry, so borders and graphs line up exactly.' },
  { icon: 'Aa', title: 'Nerd Font included', body: 'JetBrains Mono Nerd Font is embedded. Icons in nvim, starship and lazygit work with nothing installed.' },
  { icon: '◎', title: 'Smart titles', body: 'Tabs and panes show the program and its folder, like “Node ~/code/nex”, or the title the program sets.' },
  { icon: '✋', title: 'Touch and pinch', body: 'Pinch, or Ctrl+wheel (Cmd+wheel on a Mac), to zoom. On a touchscreen, swipe up and down to scroll and sideways to go to the next or previous tab.' },
  { icon: '↻', title: 'Config that just applies', body: 'A Ghostty-style key = value file. Save it and the running window updates, no restart.' },
  { icon: '◐', title: 'Every color', body: '16, 256 and 24-bit truecolor, plus bold, italic, double underline, strikethrough, dim and inverse.' },
  { icon: '🦀', title: 'Rust all the way', body: 'Built on alacritty_terminal, the VT engine behind Alacritty and Zed, with egui for rendering.' },
]
