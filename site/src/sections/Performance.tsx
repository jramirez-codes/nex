import Reveal from '../components/Reveal'
import { BENCH, DISPLAY, METRICS, ranked, runsLabel } from '../data/bench'
import { BENCH_DOC } from '../links'
import './performance.css'

function Chart({ metricKey, format }: { metricKey: string; format: (v: number) => string }) {
  const rows = ranked(metricKey)
  const max = Math.max(...rows.map(r => r.value))
  return (
    <table className="bars">
      <tbody>
        {rows.map(r => (
          <tr key={r.term} className={r.term === 'nex' ? 'is-nex' : ''}>
            <th scope="row">{DISPLAY[r.term] ?? r.term}</th>
            <td>
              <div className="bar-track" title={`${DISPLAY[r.term] ?? r.term}: ${format(r.value)}`}>
                <div className="bar-area">
                  <div className="bar" style={{ width: `${Math.max(2, (r.value / max) * 100)}%` }} />
                </div>
                <span className="bar-value">{format(r.value)}</span>
              </div>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}

export default function Performance() {
  const { machine, versions } = BENCH
  return (
    <section className="section" id="performance">
      <Reveal className="section-head">
        <p className="eyebrow">Performance</p>
        <h2 className="title">Measured, not claimed.</h2>
        <p className="lede">
          The same benchmark run against every terminal on the same machine.
          Each number is the median of {runsLabel()}. Shorter bars are better.
        </p>
      </Reveal>

      <div className="perf-grid">
        {METRICS.map((m, i) => (
          <Reveal key={m.key} delay={i * 90} className="glass perf-card">
            <h3>{m.title}</h3>
            <p className="perf-what">{m.what}</p>
            <Chart metricKey={m.key} format={m.format} />
          </Reveal>
        ))}
      </div>

      <p className="perf-note">
        {machine.cpu} · {machine.gpu} · Linux {machine.kernel} (Wayland).{' '}
        {Object.values(versions).join(', ')}. Default settings for each terminal.{' '}
        <a href={BENCH_DOC}>How it was measured, and how to run it yourself →</a>
      </p>
    </section>
  )
}
