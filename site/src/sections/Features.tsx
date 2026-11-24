import DemoVideo, { hasDemo } from '../components/DemoVideo'
import Points from '../components/Points'
import PlatformToggle from '../components/PlatformToggle'
import Reveal from '../components/Reveal'
import { DETAILS, SPOTLIGHT } from '../data/features'
import { desktopOf, usePlatform } from '../platform'
import './features.css'

export default function Features() {
  const platform = desktopOf(usePlatform())
  // A spotlight whose clip hasn't been recorded is left out rather than shown without one.
  const spots = SPOTLIGHT.filter(f => !f.shot || hasDemo(f.shot.name))
  return (
    <section className="section" id="features">
      <Reveal className="section-head">
        <p className="eyebrow">Features</p>
        <h2 className="title">Everything you reach for.{' '}<br />Already there.</h2>
        <p className="lede">
          nex looks and feels like a native app on macOS, Linux and Android, and does the jobs you
          would normally bolt on with tmux and a file manager.
        </p>
        <div className="features-toggle">
          <span>Show shortcuts for</span>
          <PlatformToggle />
        </div>
      </Reveal>

      {spots.map(f => (
        <div key={f.title} className="spot">
          <Reveal className="spot-text">
            <p className="eyebrow">{f.eyebrow}</p>
            <h3 className="spot-title">{f.title}</h3>
            <p className="spot-body">{f.body}</p>
            {f.keys && <p className="spot-keys">{f.keys[platform].map(k => <kbd key={k}>{k}</kbd>)}</p>}
          </Reveal>
          {f.shot && (
            <Reveal>
              <DemoVideo name={f.shot.name} label={f.shot.alt} width={1000} height={625} />
            </Reveal>
          )}
        </div>
      ))}

      <h3 className="details-title">And the details.</h3>
      <Points items={DETAILS} />
    </section>
  )
}
