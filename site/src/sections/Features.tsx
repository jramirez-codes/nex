import Reveal from '../components/Reveal'
import { DETAILS, SPOTLIGHT } from '../data/features'
import DemoVideo from '../components/DemoVideo'
import PlatformToggle from '../components/PlatformToggle'
import { usePlatform } from '../platform'
import './features.css'

export default function Features() {
  const platform = usePlatform()
  return (
    <section className="section" id="features">
      <Reveal className="section-head">
        <p className="eyebrow">Features</p>
        <h2 className="title">Everything you reach for.{' '}<br />Already there.</h2>
        <p className="lede">nex looks and feels like a native desktop app on macOS and Linux, and does the jobs you would normally bolt on with tmux and a file manager.</p>
        <div className="features-toggle">
          <span>Show shortcuts for</span>
          <PlatformToggle />
        </div>
      </Reveal>

      {SPOTLIGHT.map((f, i) => (
        <Reveal key={f.title} className={`spot ${i % 2 ? 'flip' : ''}`}>
          <div className="spot-text">
            <p className="eyebrow">{f.eyebrow}</p>
            <h3 className="spot-title">{f.title}</h3>
            <p className="spot-body">{f.body}</p>
            {f.keys && <p className="spot-keys">{f.keys[platform].map(k => <kbd key={k}>{k}</kbd>)}</p>}
          </div>
          {f.shot && (
            <div className="shot">
              <DemoVideo name={f.shot.name} label={f.shot.alt} width={1000} height={625} />
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
