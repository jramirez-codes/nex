import Reveal from '../components/Reveal'
import { BENCH, DISPLAY, METRICS, ranked, runsLabel } from '../data/bench'
import { ROWS, TERMINALS, type Cell } from '../data/compare'
import { BENCH_DOC } from '../links'
import './compare.css'

/** The bench.json key for each column's terminal. */
const BENCH_KEY: Record<string, string> = Object.fromEntries(Object.entries(DISPLAY).map(([k, v]) => [v, k]))

function Mark({ cell }: { cell: Cell }) {
  if (cell === true) return <span className="yes" aria-label="Yes">●</span>
  if (cell === false) return <span className="no" aria-label="No">–</span>
  return <span className="partial">{cell}</span>
}

/** One table, like a phone lineup's compare page: measured numbers first, then features. */
export default function Compare() {
  const { machine, versions } = BENCH
  return (
    <section className="section" id="specs" aria-labelledby="specs-title">
      <Reveal className="chapter-head">
        <p className="eyebrow accent-violet">Compare</p>
        <h2 className="chapter-title" id="specs-title">One app{' '}<br />instead of three.</h2>
        <p className="lede">Other terminals leave layout to tmux and files to another window. nex does both, and measures up.</p>
      </Reveal>
      <Reveal className="compare-wrap">
        <table className="compare" id="compare">
          <thead>
            <tr>
              <th scope="col"><span className="sr-only">Feature</span></th>
              {TERMINALS.map(t => <th key={t} scope="col" className={t === 'nex' ? 'is-nex' : ''}>{t}</th>)}
            </tr>
          </thead>
          <tbody id="performance">
            <tr className="group"><th scope="colgroup" colSpan={TERMINALS.length + 1}>Performance</th></tr>
            {METRICS.map(m => {
              const best = ranked(m.key)[0]?.term
              return (
                <tr key={m.key}>
                  <th scope="row">{m.title}<span className="row-what">{m.what}</span></th>
                  {TERMINALS.map(t => {
                    const key = BENCH_KEY[t]
                    const v = BENCH.metrics[m.key]?.[key]
                    return (
                      <td key={t} className={`${t === 'nex' ? 'is-nex' : ''} ${key === best ? 'best' : ''}`}>
                        {v === undefined ? <span className="no" aria-label="Not measured">–</span> : m.format(v)}
                      </td>
                    )
                  })}
                </tr>
              )
            })}
          </tbody>
          <tbody>
            <tr className="group"><th scope="colgroup" colSpan={TERMINALS.length + 1}>Features</th></tr>
            {ROWS.map(r => (
              <tr key={r.label}>
                <th scope="row">{r.label}</th>
                {r.cells.map((c, i) => <td key={i} className={i === 0 ? 'is-nex' : ''}><Mark cell={c} /></td>)}
              </tr>
            ))}
          </tbody>
        </table>
      </Reveal>
      <p className="compare-hint" aria-hidden="true">Swipe to see every terminal →</p>
      <p className="note">
        Lower is better; the best in each row is bold. Median of {runsLabel()} on the same machine:{' '}
        {machine.cpu} · {machine.gpu} · Linux {machine.kernel} (Wayland). {Object.values(versions).join(', ')}.
        Default settings for each. macOS numbers are on the way.{' '}
        <a href={BENCH_DOC}>How it was measured <span aria-hidden="true">›</span></a>
      </p>
    </section>
  )
}
