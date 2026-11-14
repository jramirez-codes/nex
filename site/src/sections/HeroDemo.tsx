import { useEffect, useState } from 'react'
import TerminalWindow, { Pane } from '../components/TerminalWindow'
import { FILES, SCRIPT, type Span } from './heroScript'

const reduced = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches

const spans = (parts: Span[]) => parts.map(([t, c], i) => <span key={i} className={c}>{t}</span>)

const Prompt = () => <><span className="c-violet bold">nex</span> <span className="c-cyan"> main</span> <span className="c-green">❯</span> </>

/** Plays SCRIPT: `line` lines are complete, `chars` characters of the next command are typed. */
function useSession() {
  const [pos, setPos] = useState(() => (reduced() ? { line: SCRIPT.length, chars: 0 } : { line: 0, chars: 0 }))

  useEffect(() => {
    if (reduced()) return
    const cur = SCRIPT[pos.line]
    let delay: number
    let next: typeof pos
    if (!cur) {
      delay = 0
      next = { line: 0, chars: 0 }
    } else if (cur.cmd && pos.chars < cur.cmd.length) {
      delay = 45 + Math.random() * 60
      next = { line: pos.line, chars: pos.chars + 1 }
    } else {
      delay = cur.wait ?? 120
      next = { line: pos.line + 1, chars: 0 }
    }
    const t = setTimeout(() => setPos(next), delay)
    return () => clearTimeout(t)
  }, [pos])

  return pos
}

/** A btop-style braille graph: every character packs 2 columns × 4 rows of dots. */
function useGraph(cols: number, rows: number) {
  const max = rows * 4
  const step = (v: number) => Math.max(1, Math.min(max, v + Math.round((Math.random() - 0.48) * 6)))
  const [vals, setVals] = useState(() => {
    const out = [max / 2]
    for (let i = 1; i < cols * 2; i++) out.push(step(out[i - 1]))
    return out
  })
  useEffect(() => {
    if (reduced()) return
    const t = setInterval(() => setVals(v => [...v.slice(1), step(v[v.length - 1])]), 380)
    return () => clearInterval(t)
  }, [])

  const LEFT = [6, 2, 1, 0]
  const RIGHT = [7, 5, 4, 3]
  const lines: string[] = []
  for (let r = rows - 1; r >= 0; r--) {
    let s = ''
    for (let c = 0; c < cols; c++) {
      let bits = 0
      const fill = (v: number, order: number[]) => {
        const n = Math.max(0, Math.min(4, v - r * 4))
        for (let k = 0; k < n; k++) bits |= 1 << order[k]
      }
      fill(vals[c * 2], LEFT)
      fill(vals[c * 2 + 1], RIGHT)
      s += String.fromCharCode(0x2800 + bits)
    }
    lines.push(s)
  }
  return { lines, pct: Math.round((vals[vals.length - 1] / max) * 100) }
}

const GRAPH_COLORS = ['c-red', 'c-orange', 'c-yellow', 'c-green', 'c-green']

export default function HeroDemo() {
  const { line, chars } = useSession()
  const graph = useGraph(30, 5)

  const shown = SCRIPT.slice(0, line).flatMap((l, i) => {
    if (l.cmd) return [<div key={i}><Prompt />{l.cmd}</div>]
    return [<div key={i}>{spans(l.out ?? [])}</div>]
  })
  const cur = SCRIPT[line]
  const typing = cur?.cmd ? cur.cmd.slice(0, chars) : ''

  return (
    <TerminalWindow
      tabs={[
        { icon: 'N', label: 'Node ~/code/nex' },
        { icon: '󰊢', label: 'Fish ~/code/nex' },
        { icon: '', label: 'Files ~/code/nex' },
      ]}
      active={1}
    >
      <Pane title="Fish ~/code/nex" className="tall" focused>
        {shown.slice(-16)}
        {(!cur || cur.cmd) && <div><Prompt />{typing}<span className="cursor" /></div>}
      </Pane>
      <Pane title="Btop ~">
        <div className="c-dim">cpu <span className="c-text">{graph.pct}%</span>  ryzen 7 · 16 threads</div>
        {graph.lines.map((g, i) => <div key={i} className={GRAPH_COLORS[i]}>{g}</div>)}
      </Pane>
      <Pane title="Files ~/code/nex">
        {FILES.map((f, i) => <div key={i} className={i === 2 ? 'row-sel' : ''}>{spans(f)}</div>)}
      </Pane>
    </TerminalWindow>
  )
}
