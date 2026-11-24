import type { Accent } from '../components/Chapter'

export interface FeatureChapter {
  id: string
  eyebrow: string
  accent: Accent
  title: [string, string]
  lede: string
  keys: { linux: string[]; mac: string[] }
  clip: { name: string; label: string } // public/demo/<name>.mp4 + .jpg poster
}

/** The feature chapters after Agents and SSH. Copy follows README.md and CHANGELOG.md. */
export const CHAPTERS: FeatureChapter[] = [
  {
    id: 'splits',
    eyebrow: 'Tabs & splits',
    accent: 'blue',
    title: ['Every split', 'is a card.'],
    lede: 'Split right or down and the new pane slides out of the old one. Zoom it, drag the gaps, and every pane’s header says what runs there.',
    keys: { linux: ['Ctrl+Shift+O', 'Ctrl+Shift+E', 'Ctrl+Shift+Enter'], mac: ['Cmd+D', 'Cmd+Shift+D', 'Cmd+Shift+Enter'] },
    clip: { name: 'splits', label: 'Splitting a tab right and down, zooming the nvim pane, then resizing and equalizing the splits' },
  },
  {
    id: 'files',
    eyebrow: 'Files & Changes',
    accent: 'orange',
    title: ['Your files.', 'Your changes.'],
    lede: 'One shortcut opens a file explorer at the pane’s folder. Another shows what git sees changed, as hunk cards. Both work over ssh.',
    keys: { linux: ['Ctrl+E', 'Ctrl+G', 'Ctrl+H'], mac: ['Cmd+E', 'Cmd+G', 'Cmd+H'] },
    clip: { name: 'explorer', label: 'Ctrl+E opens a file explorer beside the shell; three files are selected, grid view toggled, and a shell split off in src' },
  },
  {
    id: 'search',
    eyebrow: 'Search',
    accent: 'green',
    title: ['Find anything', 'you scrolled past.'],
    lede: 'A floating search field highlights every match in the scrollback and steps through them. Hold Ctrl (Cmd on a Mac) and click a link to open it.',
    keys: { linux: ['Ctrl+Shift+F'], mac: ['Cmd+F'] },
    clip: { name: 'search', label: 'Scrollback search highlighting every match of “pane” and stepping through them' },
  },
]

/** Everything else worth knowing, as a plain list. Keep it to 12. */
export const DETAILS: { title: string; body: string }[] = [
  { title: 'Agents that talk', body: 'nex-host spawn starts a helper, send asks it something, read shows its screen and done reports back.' },
  { title: 'Every pane on a host', body: 'nex --remote=user@host puts every pane on that host, as a session that reconnects when the network drops.' },
  { title: 'Picks up where you left off', body: 'A plain nex brings back every tab and split, resumes Claude Code and reattaches agents on your servers.' },
  { title: 'Your mic, on the server', body: 'Share your microphone with an ssh host, so Claude Code’s voice mode works there. Experimental.' },
  { title: 'Diffs in your colors', body: 'The Changes pane highlights each diff for its language, in your terminal palette, with changed words tinted.' },
  { title: 'Tabs that tap your shoulder', body: 'A bell or a desktop notification in a tab you aren’t looking at lights a dot on it.' },
  { title: 'Drag panes around', body: 'Drag a pane by its header onto another to swap them, onto an edge to move it, or onto the title bar for a tab of its own.' },
  { title: 'Drawn by the GPU', body: 'Each pane is one mesh of cached glyphs, and idle panes reuse their last frame.' },
  { title: 'Nerd Font included', body: 'JetBrains Mono Nerd Font is embedded, and box-drawing glyphs are drawn as geometry, so TUIs line up.' },
  { title: 'Config that just applies', body: 'A Ghostty-style key = value file. Save it and the running window updates, no restart.' },
  { title: 'Mac keys, done right', body: 'Cmd shortcuts follow Ghostty’s macOS defaults. Ctrl+C always reaches the shell, and Option types accents.' },
  { title: 'Phones that keep up', body: 'On Android every pane is a session on your server: switch apps or lose signal, and it comes back as it was.' },
]
