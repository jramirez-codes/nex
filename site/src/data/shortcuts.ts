/** The everyday shortcuts, per OS. Mirrors the Keybindings table in README.md (src/app/keymap.rs). */
export const SHORTCUTS: { action: string; linux: string; mac: string }[] = [
  { action: 'New tab', linux: 'Ctrl+Shift+T', mac: 'Cmd+T' },
  { action: 'Split right', linux: 'Ctrl+Shift+O', mac: 'Cmd+D' },
  { action: 'Split down', linux: 'Ctrl+Shift+E', mac: 'Cmd+Shift+D' },
  { action: 'File explorer here', linux: 'Ctrl+E', mac: 'Cmd+E' },
  { action: 'Changes (git diff)', linux: 'Ctrl+G', mac: 'Cmd+G' },
  { action: 'Agents', linux: 'Ctrl+Shift+A', mac: 'Cmd+Shift+A' },
  { action: 'Pin pane to every tab', linux: 'Ctrl+Shift+P', mac: 'Cmd+Shift+P' },
  { action: 'Move between splits', linux: 'Ctrl+Alt+Arrows', mac: 'Cmd+Option+Arrows' },
  { action: 'Swap with neighbor', linux: 'Alt+Shift+Arrows', mac: 'Cmd+Shift+Option+Arrows' },
  { action: 'Zoom pane', linux: 'Ctrl+Shift+Enter', mac: 'Cmd+Shift+Enter' },
  { action: 'Next / previous tab', linux: 'Ctrl+Shift+Right, Ctrl+Shift+Left', mac: 'Cmd+Shift+], Cmd+Shift+[' },
  { action: 'Go to tab N', linux: 'Alt+1…9', mac: 'Cmd+1…9' },
  { action: 'Share mic over ssh', linux: 'Ctrl+Shift+M', mac: 'Cmd+Shift+M' },
  { action: 'Search scrollback', linux: 'Ctrl+Shift+F', mac: 'Cmd+F' },
  { action: 'Copy / paste', linux: 'Ctrl+Shift+C, Ctrl+Shift+V', mac: 'Cmd+C, Cmd+V' },
  { action: 'Close pane', linux: 'Ctrl+Shift+W', mac: 'Cmd+W' },
]
