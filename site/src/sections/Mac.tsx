import type { CSSProperties } from 'react'
import Reveal from '../components/Reveal'
import { RELEASES } from '../links'
import { setPlatform } from '../platform'
import './mac.css'

/** The everyday Cmd shortcuts, drawn as keycaps. Mirrors the macOS table in src/app/keymap.rs. */
const CAPS = [
  { key: 'T', what: 'New tab' },
  { key: 'D', what: 'Split right' },
  { key: 'E', what: 'Files' },
  { key: 'F', what: 'Search' },
]

/** Copy follows CHANGELOG.md 0.7.0 / 0.7.1. Keep it honest about what isn't there yet. */
const POINTS = [
  { title: 'Universal app', body: 'One nex.app runs natively on Apple Silicon and Intel Macs. Nothing to install from Homebrew.' },
  { title: 'Native window', body: 'Traffic-light buttons, “nex” in the menu bar, and a first shell that opens in your home folder from Finder or the Dock.' },
  { title: 'Mac keys, done right', body: 'Cmd shortcuts follow Ghostty’s macOS defaults. Ctrl+C always reaches the shell, and Option types accents as usual.' },
  { title: 'Same nex everywhere', body: 'Tabs, splits, the file explorer and your ~/.config/nex/config work the same on macOS and Linux.' },
]

export default function Mac() {
  return (
    <section className="section mac" id="mac">
      <Reveal className="glass mac-card">
        <div className="mac-intro">
          <p className="eyebrow">New in 0.7 · macOS</p>
          <h2 className="title">Now at home{' '}<br />on your Mac.</h2>
          <p className="lede">
            The same tabs, splits and file explorer you get on Linux, in a native-feeling Mac app
            with the Cmd shortcuts your fingers already know.
          </p>
          <div className="mac-actions">
            <a className="btn btn-primary" href={RELEASES}>Download nex.app</a>
            <a className="btn btn-ghost" href="#install" onClick={() => setPlatform('mac')}>Install steps</a>
          </div>
        </div>

        <div className="mac-caps">
          {CAPS.map((c, i) => (
            <div key={c.key} className="mac-cap" style={{ '--press-delay': `${i * 0.9}s` } as CSSProperties}>
              <div className="cap-keys" aria-hidden="true">
                <span className="cap cap-cmd"><small>command</small>⌘</span>
                <span className="cap">{c.key}</span>
              </div>
              <span className="cap-what"><span className="sr-only">Cmd+{c.key}: </span>{c.what}</span>
            </div>
          ))}
        </div>
      </Reveal>

      <div className="mac-points">
        {POINTS.map((p, i) => (
          <Reveal key={p.title} delay={i * 70} className="mac-point">
            <h3>{p.title}</h3>
            <p>{p.body}</p>
          </Reveal>
        ))}
      </div>

      <p className="mac-note">
        Not notarized yet, so macOS asks you to clear the download flag once.
        Dragging files out of nex to Finder is still to come.
      </p>
    </section>
  )
}
