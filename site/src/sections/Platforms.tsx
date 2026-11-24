import Icon, { type IconName } from '../components/Icon'
import Reveal from '../components/Reveal'
import { RELEASES } from '../links'
import { IOS_SRC, type Platform, setPlatform } from '../platform'
import './platforms.css'

/** Copy follows README.md "Installation" and "Phone apps". The phone tile stays text only. */
const TILES: { id: Platform; anchor: string; name: string; icon: IconName; line: string; cta: string }[] = [
  {
    id: 'mac',
    anchor: 'mac',
    name: 'macOS',
    icon: 'apple',
    line: 'One universal nex.app for Apple Silicon and Intel, with the Cmd keys you already know.',
    cta: 'Download nex.app',
  },
  {
    id: 'linux',
    anchor: 'linux',
    name: 'Linux',
    icon: 'linux',
    line: 'Wayland or X11, on x86_64 and aarch64, as a .deb or a tarball.',
    cta: 'Download for Linux',
  },
  {
    id: 'android',
    anchor: 'mobile',
    name: 'Android',
    icon: 'android',
    line: 'nex’s window on your phone, with every pane on your server. Panes survive network drops.',
    cta: 'Download the APK',
  },
]

export default function Platforms() {
  return (
    <section className="section platforms" id="platforms" aria-labelledby="platforms-title">
      <Reveal className="chapter-head">
        <p className="eyebrow accent-blue">Available on</p>
        <h2 className="chapter-title" id="platforms-title">Mac. Linux.{' '}<br />And your phone.</h2>
        <p className="lede">The same tabs, splits and Agents pane, wherever you work.</p>
      </Reveal>
      <ul className="tiles">
        {TILES.map((t, i) => (
          <Reveal as="li" key={t.id} delay={i * 80} className="tile">
            <span className="tile-anchor" id={t.anchor} />
            <Icon name={t.icon} size={40} />
            <h3>{t.name}</h3>
            <p>{t.line}</p>
            <a href={RELEASES}>{t.cta} <span aria-hidden="true">›</span></a>
            <a href="#install" onClick={() => setPlatform(t.id)}>Install steps <span aria-hidden="true">›</span></a>
          </Reveal>
        ))}
      </ul>
      <p className="note">
        The iPhone app is a beta with no download yet: <a href={IOS_SRC}>build it from nex-ios</a>.
        nex.app isn’t notarized yet, so macOS asks you to clear the download flag once.
      </p>
    </section>
  )
}
