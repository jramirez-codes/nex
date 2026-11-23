import Reveal from '../components/Reveal'
import { RELEASES } from '../links'
import { IOS_SRC, setPlatform } from '../platform'
import './mobile.css'

/**
 * Copy follows nex-android/README.md, nex-ios/README.md, README.md's "Command line" (--remote)
 * and CHANGELOG.md 0.12.0–0.12.2. The iPhone app is a beta with no download: keep it that way
 * here until a release ships one.
 */
const POINTS = [
  {
    title: 'Connect once',
    body: 'Type your server and password on the connect screen. The app adds a key for this phone to the host, set to expire after 30 days, and logs in with it from then on.',
  },
  {
    title: 'Panes that survive the network',
    body: 'Every pane is a nex-host session on the server. Switch apps, change networks or lose signal, and the pane reconnects with the screen as it was.',
  },
  {
    title: 'A keyboard button that does more',
    body: 'Drag the round glass button as a joystick for the arrow keys. Double-tap it to arm Ctrl for the next key. Taps, arrows and Ctrl give a light haptic tick.',
  },
  {
    title: 'Swipe typing',
    body: 'Glide typing on the on-screen keyboard types into the terminal, and the keyboard’s corrections replace the word.',
  },
  {
    title: 'Hold to talk',
    body: 'Hold the keyboard button to talk to Claude Code’s voice mode. The phone’s mic is shared with the host and Space is held for you until you let go.',
  },
  {
    title: 'Agents in your pocket',
    body: 'The Agents pane lists the agents on the server over the app’s own connection. Open one to attach; close it and the agent keeps running.',
  },
]

export default function Mobile() {
  return (
    <section className="section mobile" id="mobile">
      <Reveal className="glass mobile-card">
        <div className="mobile-intro">
          <p className="eyebrow">Android · iPhone beta</p>
          <h2 className="title">Your server,{' '}<br />in your pocket.</h2>
          <p className="lede">
            The phone runs only nex’s window; every pane works on an ssh host such as your home
            server. Tabs, the Agents pane, Files and Changes, with touch made for a terminal.
          </p>
          <div className="mobile-actions">
            <a className="btn btn-primary" href={RELEASES}>Download the APK</a>
            <a className="btn btn-ghost" href="#install" onClick={() => setPlatform('android')}>Install steps</a>
          </div>
          <p className="mobile-file">
            Every release has <code>nex-v*-android-arm64.apk</code>.
          </p>
        </div>

        {/* A phone running nex: a Claude Code pane over ssh, with the glass keyboard button. */}
        <div
          className="mobile-phone"
          role="img"
          aria-label="nex on a phone: a terminal pane on the home server running Claude Code, with the round keyboard button armed for Ctrl in the corner"
        >
          <div className="mobile-screen" aria-hidden="true">
            <div className="mobile-tabs">
              <span className="mobile-tab on">claude ~/app</span>
              <span className="mobile-tab">Agents</span>
            </div>
            <div className="mobile-head"><span className="mobile-dot" />home.lan <span>~/app</span></div>
            <pre className="mobile-body">
              <span className="p">you@home</span>:<span className="d">~/app</span>$ claude{'\n'}
              {'> '}fix the flaky login test{'\n'}
              <span className="mobile-dim">● Reading tests/login.rs…</span>{'\n'}
              <span className="mobile-dim">● Working (12s)</span>
            </pre>
            <div className="mobile-key"><span>⌃</span></div>
          </div>
        </div>
      </Reveal>

      <div className="mobile-points">
        {POINTS.map((p, i) => (
          <Reveal key={p.title} delay={(i % 3) * 70} className="mobile-point">
            <h3>{p.title}</h3>
            <p>{p.body}</p>
          </Reveal>
        ))}
      </div>

      <Reveal className="mobile-ios">
        <div className="mobile-ios-text">
          <h3>iPhone <span className="mobile-beta">Beta</span></h3>
          <p>
            The iPhone app works the same as the Android one, drawn with Metal. There is no
            download yet: build it yourself on a Mac with Xcode 15 or newer, and an Apple
            Developer account to put it on a real iPhone.
          </p>
        </div>
        <a className="btn btn-ghost" href={IOS_SRC}>Build it from nex-ios →</a>
      </Reveal>

      <p className="mobile-note">
        Same idea on a desktop: <code>nex --remote=user@host</code> puts every pane on that host.
      </p>
    </section>
  )
}
