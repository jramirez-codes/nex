import Keys from '../components/Keys'
import PlatformToggle from '../components/PlatformToggle'
import Reveal from '../components/Reveal'
import { SHORTCUTS } from '../data/shortcuts'
import { REPO } from '../links'
import { desktopOf, usePlatform } from '../platform'
import './shortcuts.css'

export default function Shortcuts() {
  const platform = desktopOf(usePlatform())
  return (
    <section className="section" id="shortcuts">
      <Reveal className="chapter-head">
        <p className="eyebrow">Shortcuts</p>
        <h2 className="chapter-title">Keys that feel native.</h2>
        <p className="lede">Cmd on a Mac, Ctrl+Shift on Linux, so plain Ctrl stays with your shell.</p>
      </Reveal>
      <Reveal className="shortcuts">
        <div className="shortcuts-bar">
          <PlatformToggle />
        </div>
        <dl className="keylist">
          {SHORTCUTS.map(s => (
            <div key={s.action}>
              <dt>{s.action}</dt>
              <dd><Keys combo={s[platform]} /></dd>
            </div>
          ))}
        </dl>
        <p className="shortcuts-more">
          <a href={`${REPO}#keybindings`}>Every shortcut, including the file explorer <span aria-hidden="true">›</span></a>
        </p>
      </Reveal>
    </section>
  )
}
