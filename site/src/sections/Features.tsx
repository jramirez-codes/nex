import Points from '../components/Points'
import Reveal from '../components/Reveal'
import { DETAILS } from '../data/features'
import './features.css'

export default function Features() {
  return (
    <section className="section" id="features" aria-labelledby="features-title">
      <Reveal className="chapter-head">
        <p className="eyebrow">Features</p>
        <h2 className="chapter-title" id="features-title">And the details.</h2>
        <p className="lede">Everything else you would bolt on with tmux and a file manager, already there.</p>
      </Reveal>
      <Points items={DETAILS} />
    </section>
  )
}
