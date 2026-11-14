import Reveal from '../components/Reveal'
import { DETAILS, SPOTLIGHT } from '../data/features'
import { asset } from '../links'
import './features.css'

export default function Features() {
  return (
    <section className="section" id="features">
      <Reveal className="section-head">
        <p className="eyebrow">Features</p>
        <h2 className="title">Everything you reach for.<br />Already there.</h2>
        <p className="lede">nex looks and feels like a native desktop app, and does the jobs you would normally bolt on with tmux and a file manager.</p>
      </Reveal>

      {SPOTLIGHT.map((f, i) => (
        <Reveal key={f.title} className={`spot ${i % 2 ? 'flip' : ''}`}>
          <div className="spot-text">
            <p className="eyebrow">{f.eyebrow}</p>
            <h3 className="spot-title">{f.title}</h3>
            <p className="spot-body">{f.body}</p>
            {f.keys && <p className="spot-keys">{f.keys.map(k => <kbd key={k}>{k}</kbd>)}</p>}
          </div>
          {f.shot && (
            <div className="shot">
              <img src={asset(f.shot.src)} alt={f.shot.alt} loading="lazy" width={1440} height={880} />
            </div>
          )}
        </Reveal>
      ))}

      <div className="details">
        {DETAILS.map((d, i) => (
          <Reveal key={d.title} delay={(i % 4) * 70} className="glass detail">
            <div className="detail-icon" aria-hidden="true">{d.icon}</div>
            <h3>{d.title}</h3>
            <p>{d.body}</p>
          </Reveal>
        ))}
      </div>
    </section>
  )
}
