import Reveal from '../components/Reveal'
import { METRICS, lead } from '../data/bench'
import './highlights.css'

/** Big-number tiles. Benchmark tiles appear only for metrics nex actually leads. */
export default function Highlights() {
  const wins = METRICS.flatMap(m => {
    const l = lead(m.key)
    return l ? [{ big: `${l.ratio.toFixed(1)}×`, small: `${m.verb} ${l.over}, the next best` }] : []
  })
  const tiles = [
    ...wins,
    { big: '0', small: 'plugins or multiplexers needed for tabs, splits and files' },
    { big: '100%', small: 'Rust, open source under the MIT license' },
  ].slice(0, 4)

  return (
    <section className="section highlights" aria-label="Highlights">
      {tiles.map((t, i) => (
        <Reveal key={t.small} delay={i * 80} className="glass tile">
          <div className="tile-big gradient-text">{t.big}</div>
          <div className="tile-small">{t.small}</div>
        </Reveal>
      ))}
    </section>
  )
}
