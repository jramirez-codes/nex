import Keys from '../components/Keys'
import PlatformToggle from '../components/PlatformToggle'
import Reveal from '../components/Reveal'
import { SHORTCUTS } from '../data/shortcuts'
import { REPO } from '../links'
import { usePlatform } from '../platform'
import './shortcuts.css'

export default function Shortcuts() {
  const platform = usePlatform()
  return (
    <section className="section" id="shortcuts">
      <Reveal className="section-head">
        <p className="eyebrow">Shortcuts</p>
        <h2 className="title">Keys that feel native.</h2>
        <p className="lede">
          nex follows Ghostty’s defaults on each OS: Cmd on a Mac, Ctrl+Shift on Linux, so the
          plain Ctrl keys stay with your shell.
        </p>
      </Reveal>
      <Reveal className="glass shortcuts">
        <div className="shortcuts-bar">
          <PlatformToggle />
        </div>
        <dl className="shortcut-list">
          {SHORTCUTS.map(s => (
            <div key={s.action} className="shortcut">
              <dt>{s.action}</dt>
              <dd><Keys combo={s[platform]} /></dd>
            </div>
          ))}
        </dl>
        <p className="shortcuts-more">
          <a href={`${REPO}#keybindings`}>Every shortcut, including the file explorer →</a>
        </p>
      </Reveal>
    </section>
  )
}
