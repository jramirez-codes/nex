export interface Feature {
  eyebrow: string
  title: string
  body: string
  keys?: string[]
  shot?: { name: string; alt: string } // public/demo/<name>.mp4 + .jpg poster
}

/** The big alternating feature rows. Copy follows README.md and CHANGELOG.md. */
export const SPOTLIGHT: Feature[] = [
  {
    eyebrow: 'Tabs and splits, built in',
    title: 'Every split is a card.',
    body: 'Tabs live in the title bar on a segmented track with a glass pill that glides to the one you pick. Split right or down and the new pane slides out of the old one’s edge; close one and its neighbor grows into the space. Zoom a pane, drag the gaps to resize. Each pane has its own header with what is running and where.',
    keys: ['Ctrl+Shift+O', 'Ctrl+Shift+E', 'Ctrl+Shift+Enter'],
    shot: { name: 'splits', alt: 'Splitting a tab right and down, zooming the nvim pane, then resizing and equalizing the splits' },
  },
  {
    eyebrow: 'Only in nex',
    title: 'A file explorer, right next to your shell.',
    body: 'Press Ctrl+E and a file explorer splits off the current pane, open at its folder. List or grid view, multi-select, copy, cut and paste. Drag files onto a terminal to type their quoted paths, or split the explorer to start a shell in the folder you are looking at.',
    keys: ['Ctrl+E', 'Ctrl+G', 'Ctrl+H'],
    shot: { name: 'explorer', alt: 'Ctrl+E opens a file explorer beside the shell; three files are selected, grid view toggled, and a shell split off in src' },
  },
  {
    eyebrow: 'Scrollback search and links',
    title: 'Find anything you have scrolled past.',
    body: 'A floating search field highlights every match in the scrollback and steps through them. Hold Ctrl to underline URLs and OSC 8 hyperlinks, and click to open them.',
    keys: ['Ctrl+Shift+F'],
    shot: { name: 'search', alt: 'Scrollback search highlighting every match of “pane” and stepping through them' },
  },
]

/** The smaller grid of details. */
export const DETAILS: { icon: string; title: string; body: string }[] = [
  { icon: '⚡', title: 'Drawn by the GPU', body: 'Each pane is one mesh of cached glyphs, and idle panes reuse their last frame, so btop or nvim stay smooth with CPU to spare.' },
  { icon: '▦', title: 'Pixel-perfect TUIs', body: 'Box-drawing, block, braille and Powerline glyphs are drawn as geometry, so borders and graphs line up exactly.' },
  { icon: 'Aa', title: 'Nerd Font included', body: 'JetBrains Mono Nerd Font is embedded. Icons in nvim, starship and lazygit work with nothing installed.' },
  { icon: '◎', title: 'Smart titles', body: 'Tabs and panes show the program and its folder, like “Node ~/code/nex”, or the title the program sets.' },
  { icon: '✋', title: 'Touch and pinch', body: 'Pinch on a touchscreen or touchpad, or Ctrl+wheel, to zoom a terminal’s font or an explorer’s listing.' },
  { icon: '↻', title: 'Config that just applies', body: 'A Ghostty-style key = value file. Save it and the running window updates, no restart.' },
  { icon: '◐', title: 'Every color', body: '16, 256 and 24-bit truecolor, plus bold, italic, double underline, strikethrough, dim and inverse.' },
  { icon: '🦀', title: 'Rust all the way', body: 'Built on alacritty_terminal, the VT engine behind Alacritty and Zed, with egui for rendering.' },
]
