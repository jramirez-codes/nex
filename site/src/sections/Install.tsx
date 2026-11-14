import { useState, type ReactNode } from 'react'
import CopyCommand from '../components/CopyCommand'
import PlatformToggle from '../components/PlatformToggle'
import Reveal from '../components/Reveal'
import { REPO, RELEASES } from '../links'
import { type Platform, usePlatform } from '../platform'
import './install.css'

interface Method {
  id: string
  label: string
  note: ReactNode
  lines: string[]
}

/** Install steps per OS, first one is the recommended. Keep in sync with README.md "Installation". */
const METHODS: Record<Platform, Method[]> = {
  mac: [
    {
      id: 'app',
      label: 'nex.app',
      note: <>Unzip <code>nex-v*-macos-app.zip</code> from the <a href={RELEASES}>latest release</a> and drag <code>nex.app</code> to Applications. It isn’t notarized yet, so clear the download flag once:</>,
      lines: ['xattr -dr com.apple.quarantine /Applications/nex.app'],
    },
    {
      id: 'tar',
      label: 'App + CLI',
      note: <>The tarball from the <a href={RELEASES}>latest release</a> installs <code>nex.app</code> and the <code>nex</code> command in <code>~/.local/bin</code>.</>,
      lines: [
        'tar xzf nex-v*-universal-macos.tar.gz',
        'xattr -dr com.apple.quarantine nex-v*-universal-macos',
        './nex-v*-universal-macos/install.sh',
      ],
    },
    {
      id: 'src',
      label: 'From source',
      note: <>Needs Rust 1.88+ and the Xcode command line tools (<code>xcode-select --install</code>). Installs <code>nex.app</code> to <code>/Applications</code>.</>,
      lines: [`git clone ${REPO}.git`, 'cd nex && ./install.sh'],
    },
  ],
  linux: [
    {
      id: 'deb',
      label: 'Debian / Ubuntu',
      note: <>Grab the <code>.deb</code> for x86_64 or aarch64 from the <a href={RELEASES}>latest release</a>, then:</>,
      lines: ['sudo apt install ./nex_*_amd64.deb'],
    },
    {
      id: 'tar',
      label: 'Any Linux',
      note: <>Download the tarball from the <a href={RELEASES}>latest release</a>. It installs to <code>~/.local</code>, with a desktop entry for your app launcher.</>,
      lines: ['tar xzf nex-v*-x86_64-linux.tar.gz', './nex-v*-x86_64-linux/install.sh'],
    },
    {
      id: 'src',
      label: 'From source',
      note: <>Needs Rust 1.88+ and the <code>libxkbcommon</code>, <code>wayland</code>, <code>mesa</code> and <code>fontconfig</code> dev packages.</>,
      lines: [`git clone ${REPO}.git`, 'cd nex && ./install.sh'],
    },
  ],
}

const SUPPORT: Record<Platform, string> = {
  mac: 'Universal build for Apple Silicon and Intel Macs.',
  linux: 'X11 or Wayland, on x86_64 and aarch64.',
}

export default function Install() {
  const platform = usePlatform()
  const methods = METHODS[platform]
  const [picked, setPicked] = useState<Record<Platform, string>>({ mac: 'app', linux: 'deb' })
  const method = methods.find(m => m.id === picked[platform]) ?? methods[0]

  return (
    <section className="section" id="install">
      <Reveal className="section-head">
        <p className="eyebrow">Install</p>
        <h2 className="title">Up and running in{' '}<br />one command.</h2>
        <p className="lede">Free and open source, for macOS and Linux.</p>
      </Reveal>
      <Reveal className="glass install">
        <div className="install-os">
          <PlatformToggle />
          <span className="install-support">{SUPPORT[platform]}</span>
        </div>
        <div className="seg" role="tablist" aria-label="Install method">
          {methods.map(m => (
            <button
              key={m.id}
              type="button"
              role="tab"
              aria-selected={m.id === method.id}
              className={m.id === method.id ? 'on' : ''}
              onClick={() => setPicked(p => ({ ...p, [platform]: m.id }))}
            >
              {m.label}
            </button>
          ))}
        </div>
        <div role="tabpanel">
          <p className="install-note">{method.note}</p>
          <CopyCommand lines={method.lines} />
        </div>
        <p className="install-foot">
          Each release ships a <code>SHA256SUMS</code> file to verify downloads. <a href={`${REPO}#installation`}>Full install guide →</a>
        </p>
      </Reveal>
    </section>
  )
}
