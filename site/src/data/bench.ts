import raw from './bench.json'

/** Measured by bench/run.sh. Every number on the site comes from here. */
export const BENCH = raw as {
  machine: { cpu: string; gpu: string; kernel: string }
  runs: Record<string, number>
  versions: Record<string, string>
  metrics: Record<string, Record<string, number>>
}

export interface Metric {
  key: string
  title: string
  what: string
  verb: string // "faster output than", used in the highlight tiles
  format: (v: number) => string
}

export const METRICS: Metric[] = [
  {
    key: 'throughput_ms',
    title: 'Output throughput',
    what: 'Time to print a 50 MB file of plain text, 24-bit color and box-drawing glyphs.',
    verb: 'faster output than',
    format: v => `${(v / 1000).toFixed(2)} s`,
  },
  {
    key: 'startup_ms',
    title: 'Launch to shell',
    what: 'From starting the terminal to its shell running its first command.',
    verb: 'faster launch than',
    format: v => `${Math.round(v)} ms`,
  },
  {
    key: 'idle_rss_mb',
    title: 'Memory at idle',
    what: 'Resident memory of the terminal process with one shell open.',
    verb: 'less memory than',
    format: v => `${Math.round(v)} MB`,
  },
]

export const DISPLAY: Record<string, string> = {
  nex: 'nex',
  alacritty: 'Alacritty',
  kitty: 'kitty',
  ghostty: 'Ghostty',
  'gnome-terminal': 'GNOME Terminal',
}

/** Results for one metric, best (lowest) first. */
export function ranked(key: string) {
  return Object.entries(BENCH.metrics[key] ?? {})
    .map(([term, value]) => ({ term, value }))
    .sort((a, b) => a.value - b.value)
}

/** When nex beats every other terminal on a metric: by how much, and over whom. */
export function lead(key: string) {
  const [first, second] = ranked(key)
  if (!first || !second || first.term !== 'nex') return null
  return { ratio: second.value / first.value, over: DISPLAY[second.term] ?? second.term }
}

/** "3 runs", or "3–10 runs" when terminals were measured a different number of times. */
export function runsLabel() {
  const counts = Object.values(BENCH.runs)
  const lo = Math.min(...counts)
  const hi = Math.max(...counts)
  return lo === hi ? `${lo} runs` : `${lo}–${hi} runs`
}
