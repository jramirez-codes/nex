import { useState } from 'react'
import CopyCommand from '../components/CopyCommand'
import Reveal from '../components/Reveal'
import { REPO, RELEASES } from '../links'
import './install.css'

const METHODS = [
  {
    id: 'deb',
    label: 'Debian / Ubuntu',
    note: <>Grab the <code>.deb</code> for x86_64 or aarch64 from the <a href={RELEASES}>latest release</a>, then:</>,
    lines: ['sudo apt install ./nex_*_amd64.deb'],
  },
  {
    id: 'tar',
    label: 'Any Linux',
    note: <>Download the tarball from the <a href={RELEASES}>latest release</a>. It installs to <code>~/.local</code>.</>,
    lines: ['tar xzf nex-v*-x86_64-linux.tar.gz', './nex-v*-x86_64-linux/install.sh'],
  },
  {
    id: 'src',
    label: 'From source',
    note: <>Needs Rust 1.88+ plus <code>libxkbcommon</code>, <code>wayland</code>, <code>mesa</code> and <code>fontconfig</code> dev packages.</>,
    lines: [`git clone ${REPO}.git`, 'cd nex && ./install.sh'],
  },
]

export default function Install() {
  const [tab, setTab] = useState(METHODS[0].id)
  const method = METHODS.find(m => m.id === tab)!
  return (
    <section className="section" id="install">
      <Reveal className="section-head">
        <p className="eyebrow">Install</p>
        <h2 className="title">Up and running in<br />one command.</h2>
        <p className="lede">Linux, X11 or Wayland. Prebuilt for x86_64 and aarch64.</p>
      </Reveal>
      <Reveal className="glass install">
        <div className="seg" role="tablist" aria-label="Install method">
          {METHODS.map(m => (
            <button key={m.id} role="tab" aria-selected={m.id === tab} className={m.id === tab ? 'on' : ''} onClick={() => setTab(m.id)}>
              {m.label}
            </button>
          ))}
        </div>
        <div role="tabpanel">
          <p className="install-note">{method.note}</p>
          <CopyCommand lines={method.lines} />
        </div>
      </Reveal>
    </section>
  )
}
