/** The everyday shortcuts, per OS. Mirrors the Keybindings table in README.md (src/app/keymap.rs). */
export const SHORTCUTS: { action: string; linux: string; mac: string }[] = [
  { action: 'New tab', linux: 'Ctrl+Shift+T', mac: 'Cmd+T' },
  { action: 'Split right', linux: 'Ctrl+Shift+O', mac: 'Cmd+D' },
  { action: 'Split down', linux: 'Ctrl+Shift+E', mac: 'Cmd+Shift+D' },
  { action: 'File explorer here', linux: 'Ctrl+E', mac: 'Cmd+E' },
  { action: 'Move between splits', linux: 'Ctrl+Alt+Arrows', mac: 'Cmd+Option+Arrows' },
  { action: 'Zoom pane', linux: 'Ctrl+Shift+Enter', mac: 'Cmd+Shift+Enter' },
  { action: 'Go to tab N', linux: 'Alt+1…9', mac: 'Cmd+1…9' },
  { action: 'Search scrollback', linux: 'Ctrl+Shift+F', mac: 'Cmd+F' },
  { action: 'Copy / paste', linux: 'Ctrl+Shift+C, Ctrl+Shift+V', mac: 'Cmd+C, Cmd+V' },
  { action: 'Close pane', linux: 'Ctrl+Shift+W', mac: 'Cmd+W' },
]
