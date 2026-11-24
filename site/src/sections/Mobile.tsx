import Icon from '../components/Icon'
import Points from '../components/Points'
import Reveal from '../components/Reveal'
import { RELEASES } from '../links'
import { IOS_SRC, setPlatform } from '../platform'

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
    <section className="section" id="mobile">
      <Reveal className="section-head">
        <p className="eyebrow">Android · iPhone beta</p>
        <h2 className="title">Your server,{' '}<br />in your pocket.</h2>
        <p className="lede">
          The phone runs only nex’s window; every pane works on an ssh host such as your home
          server. Tabs, the Agents pane, Files and Changes, with touch made for a terminal.
        </p>
        <div className="actions">
          <a className="btn btn-primary" href={RELEASES}>
            <Icon name="download" size={18} />Download the APK<Icon name="android" size={18} />
          </a>
          <a className="btn btn-link" href="#install" onClick={() => setPlatform('android')}>
            Install steps <span aria-hidden="true">›</span>
          </a>
        </div>
      </Reveal>

      <Points items={POINTS} />

      <p className="note">
        Every release has <code>nex-v*-android-arm64.apk</code>. The iPhone app is a beta drawn
        with Metal, with no download yet: <a href={IOS_SRC}>build it from nex-ios</a> on a Mac
        with Xcode 15 or newer. Same idea on a desktop: <code>nex --remote=user@host</code> puts
        every pane on that host.
      </p>
    </section>
  )
}
