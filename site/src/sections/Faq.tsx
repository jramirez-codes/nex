import Reveal from '../components/Reveal'
import { FAQ } from '../data/faq'
import './faq.css'

export default function Faq() {
  return (
    <section className="section" id="faq">
      <Reveal className="section-head">
        <p className="eyebrow">FAQ</p>
        <h2 className="title">Questions, answered.</h2>
      </Reveal>
      <Reveal className="faq">
        {FAQ.map(item => (
          <details key={item.q} className="glass faq-item">
            <summary><h3>{item.q}</h3></summary>
            <p>{item.a}</p>
          </details>
        ))}
      </Reveal>
    </section>
  )
}
