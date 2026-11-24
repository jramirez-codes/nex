import Icon from '../components/Icon'
import Keys from '../components/Keys'
import Points from '../components/Points'
import Reveal from '../components/Reveal'
import { RELEASES } from '../links'
import { setPlatform } from '../platform'

/** The everyday Cmd shortcuts. Mirrors the macOS table in src/app/keymap.rs. */
const KEYS = [
  { combo: 'Cmd+T', what: 'New tab' },
  { combo: 'Cmd+D', what: 'Split right' },
  { combo: 'Cmd+E', what: 'Files' },
  { combo: 'Cmd+G', what: 'Changes' },
  { combo: 'Cmd+Shift+A', what: 'Agents' },
  { combo: 'Cmd+F', what: 'Search' },
]

/** Copy follows CHANGELOG.md 0.7.0 / 0.7.1. Keep it honest about what isn't there yet. */
const POINTS = [
  { title: 'Universal app', body: 'One nex.app runs natively on Apple Silicon and Intel Macs. Nothing to install from Homebrew.' },
  { title: 'Native window', body: 'Traffic-light buttons, “Nex” in the menu bar, and a first shell that opens in your home folder from Finder or the Dock.' },
  { title: 'Mac keys, done right', body: 'Cmd shortcuts follow Ghostty’s macOS defaults. Ctrl+C always reaches the shell, and Option types accents as usual.' },
]

export default function Mac() {
  return (
    <section className="section" id="mac">
      <Reveal className="section-head">
        <p className="eyebrow">macOS · Apple Silicon &amp; Intel</p>
        <h2 className="title">Now at home{' '}<br />on your Mac.</h2>
        <p className="lede">
          The same tabs, splits and file explorer you get on Linux, in a native-feeling Mac app
          with the Cmd shortcuts your fingers already know.
        </p>
        <div className="actions">
          <a className="btn btn-primary" href={RELEASES}>
            <Icon name="download" size={18} />Download nex.app<Icon name="apple" size={18} />
          </a>
          <a className="btn btn-link" href="#install" onClick={() => setPlatform('mac')}>
            Install steps <span aria-hidden="true">›</span>
          </a>
        </div>
      </Reveal>

      <Reveal>
        <dl className="keylist">
          {KEYS.map(k => (
            <div key={k.combo}>
              <dt>{k.what}</dt>
              <dd><Keys combo={k.combo} /></dd>
            </div>
          ))}
        </dl>
      </Reveal>

      <Points items={POINTS} />

      <p className="note">
        Not notarized yet, so macOS asks you to clear the download flag once.
        Dragging files out of nex to Finder is still to come.
      </p>
    </section>
  )
}
