/** true = yes, false = no, string = yes/no with a qualifier. Sourced from each project's docs. */
export type Cell = boolean | string

export const TERMINALS = ['nex', 'Alacritty', 'kitty', 'Ghostty', 'GNOME Terminal'] as const

export const ROWS: { label: string; cells: Cell[] }[] = [
  { label: 'Runs on', cells: ['Linux, macOS', 'Linux, macOS, Windows', 'Linux, macOS', 'Linux, macOS', 'Linux'] },
  { label: 'GPU-rendered', cells: [true, true, true, true, false] },
  { label: 'Tabs', cells: [true, false, true, true, true] },
  { label: 'Split panes', cells: [true, false, true, true, false] },
  { label: 'Agents pane (Claude Code, Codex)', cells: [true, false, false, false, false] },
  { label: 'Agent pipelines and teams', cells: [true, false, false, false, false] },
  { label: 'Pin a pane to every tab', cells: [true, false, false, false, false] },
  { label: 'Built-in file explorer', cells: [true, false, false, false, false] },
  { label: 'Splits and tabs follow ssh sessions', cells: [true, false, 'ssh kitten', false, false] },
  { label: 'Drop or paste files into ssh sessions', cells: [true, false, false, false, false] },
  { label: 'Per-pane headers (program + folder)', cells: [true, false, false, false, false] },
  { label: 'Nerd Font icons with nothing installed', cells: [true, false, true, true, false] },
  { label: 'Config applies on save', cells: [true, true, 'Shortcut', 'Shortcut', 'GUI only'] },
]
