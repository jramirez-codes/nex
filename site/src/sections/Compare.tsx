import Reveal from '../components/Reveal'
import { ROWS, TERMINALS, type Cell } from '../data/compare'
import './compare.css'

function Mark({ cell }: { cell: Cell }) {
  if (cell === true) return <span className="yes" aria-label="Yes">●</span>
  if (cell === false) return <span className="no" aria-label="No">–</span>
  return <span className="partial">{cell}</span>
}

export default function Compare() {
  return (
    <section className="section" id="compare">
      <Reveal className="section-head">
        <p className="eyebrow">Compare</p>
        <h2 className="title">One app instead of three.</h2>
        <p className="lede">Other terminals leave layout to tmux and files to another window. nex does both, in the same GPU-drawn window.</p>
      </Reveal>
      <Reveal className="glass compare-wrap">
        <table className="compare">
          <thead>
            <tr>
              <th scope="col"><span className="sr-only">Feature</span></th>
              {TERMINALS.map(t => <th key={t} scope="col" className={t === 'nex' ? 'is-nex' : ''}>{t}</th>)}
            </tr>
          </thead>
          <tbody>
            {ROWS.map(r => (
              <tr key={r.label}>
                <th scope="row">{r.label}</th>
                {r.cells.map((c, i) => <td key={i} className={i === 0 ? 'is-nex' : ''}><Mark cell={c} /></td>)}
              </tr>
            ))}
          </tbody>
        </table>
      </Reveal>
    </section>
  )
}
