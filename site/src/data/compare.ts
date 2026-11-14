/** true = yes, false = no, string = yes/no with a qualifier. Sourced from each project's docs. */
export type Cell = boolean | string

export const TERMINALS = ['nex', 'Alacritty', 'kitty', 'Ghostty', 'GNOME Terminal'] as const

export const ROWS: { label: string; cells: Cell[] }[] = [
  { label: 'Runs on', cells: ['Linux, macOS', 'Linux, macOS, Windows', 'Linux, macOS', 'Linux, macOS', 'Linux'] },
  { label: 'GPU-rendered', cells: [true, true, true, true, false] },
  { label: 'Tabs', cells: [true, false, true, true, true] },
  { label: 'Split panes', cells: [true, false, true, true, false] },
  { label: 'Built-in file explorer', cells: [true, false, false, false, false] },
  { label: 'Per-pane headers (program + folder)', cells: [true, false, false, false, false] },
  { label: 'Nerd Font icons with nothing installed', cells: [true, false, true, true, false] },
  { label: 'Config applies on save', cells: [true, true, 'Shortcut', 'Shortcut', 'GUI only'] },
]
