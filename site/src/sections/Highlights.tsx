import Reveal from '../components/Reveal'
import { METRICS, lead } from '../data/bench'
import './highlights.css'

/** A tech-specs row of big numbers. Benchmark numbers appear only where nex actually leads. */
export default function Highlights() {
  const wins = METRICS.flatMap(m => {
    const l = lead(m.key)
    return l ? [{ big: `${l.ratio.toFixed(1)}×`, small: `${m.verb} ${l.over}, the next best`, note: m.note }] : []
  })
  const specs: { big: string; small: string; note?: string }[] = [
    ...wins,
    { big: '0', small: 'plugins or multiplexers needed for tabs, splits and files' },
    { big: '$0', small: 'Free and open source under the MIT license' },
  ].slice(0, 4)
  const notes = specs.filter(s => s.note)

  return (
    <section className="section highlights" aria-label="Highlights">
      <ul className="specs">
        {specs.map((s, i) => (
          <Reveal as="li" key={s.small} delay={i * 80} className="spec">
            <span className="spec-big">
              {s.big}
              {s.note && <sup className="spec-mark">{notes.indexOf(s) + 1}</sup>}
            </span>
            <span className="spec-small">{s.small}</span>
          </Reveal>
        ))}
      </ul>
      {notes.map((s, i) => <p key={s.small} className="specs-note">{i + 1}. {s.note}</p>)}
    </section>
  )
}
