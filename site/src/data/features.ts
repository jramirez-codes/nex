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
  {
    eyebrow: 'One window',
    title: 'A pane for every program.',
    body: 'git, btop and nvim side by side, each pane’s header saying what runs in it and where. Move between them and the tab’s icons turn with the focus; open another tab and the panes glide across.',
    keys: { linux: ['Ctrl+Alt+Arrows', 'Ctrl+Shift+T', 'Alt+1…9'], mac: ['Cmd+Option+Arrows', 'Cmd+T', 'Cmd+1…9'] },
    shot: { name: 'tour', alt: 'git log, btop and nvim in three panes, focus walking between them, then a second tab and back' },
  },
]

/** The strongest of the smaller details, as a plain list under the recordings. Keep it to 8. */
export const DETAILS: { title: string; body: string }[] = [
  { title: 'Every pane on a host', body: 'nex --remote=user@host opens a window whose every pane lives on that host, as a nex-host session that reconnects when the network drops.' },
  { title: 'Picks up where you left off', body: 'A plain nex brings back every tab and split, re-runs what each pane was running, resumes Claude Code (claude --resume) and reattaches agents on your servers.' },
  { title: 'Diffs in your colors', body: 'The Changes pane highlights each diff for its language, in colors taken from your terminal palette, with changed words tinted on top.' },
  { title: 'Tabs that tap your shoulder', body: 'A bell or a desktop notification (OSC 9, 99 or 777) in a tab you aren’t looking at lights a dot on it, and flags the dock or taskbar.' },
  { title: 'Drag panes around', body: 'Drag a pane by its header onto another to swap them, onto an edge to put it there, onto the title bar for a tab of its own, or over another tab to move it.' },
  { title: 'Drawn by the GPU', body: 'Each pane is one mesh of cached glyphs, and idle panes reuse their last frame, so btop or nvim stay smooth with CPU to spare.' },
  { title: 'Nerd Font included', body: 'JetBrains Mono Nerd Font is embedded, and box-drawing and Powerline glyphs are drawn as geometry, so TUIs line up exactly.' },
  { title: 'Config that just applies', body: 'A Ghostty-style key = value file. Save it and the running window updates, no restart.' },
]
