import Reveal from '../components/Reveal'
import { usePlatform } from '../platform'
import './ssh.css'

/** Copy follows the README's "SSH, built in" section and CHANGELOG.md 0.10.0. */
const POINTS = [
  {
    title: 'Splits stay on the server',
    body: 'Split an ssh pane or open a tab from it and you get another session on the same host, in the same folder. exit drops you back to a local shell.',
  },
  {
    title: 'Drop it, paste it, done',
    body: 'Files dropped or pasted into an ssh pane are uploaded to a fresh temp folder on the host, and nex types the remote paths. Folders work too.',
  },
  {
    title: 'One login',
    body: 'Keys and agents just work. For password hosts, nex offers to turn on connection sharing once, and everything after rides on the login you typed.',
  },
  {
    title: 'Plain ssh, nothing to install',
    body: 'No wrapper command, no server agent. nex reuses the pane’s own ssh options, with sh and tar on the other end.',
  },
]

const SHARING = `# Added by nex: ssh started in a nex pane shares its connection …
Match exec "test x$TERM_PROGRAM = xnex"
  ControlMaster auto
  ControlPath ~/.ssh/nex-%C
  ControlPersist 60`

export default function Ssh() {
  const platform = usePlatform()
  const paste = platform === 'mac' ? 'Cmd+V' : 'Ctrl+V'
  return (
    <section className="section ssh" id="ssh">
      <Reveal className="glass ssh-card">
        <div className="ssh-intro">
          <p className="eyebrow">New in 0.10 · SSH</p>
          <h2 className="title">SSH that{' '}<br />follows you.</h2>
          <p className="lede">
            Run plain <code>ssh</code> and the rest of nex comes with you: splits open on the same
            server, and files you drop or paste land there too. Paste a screenshot into a remote
            Claude Code and it just gets a path it can open.
          </p>
        </div>

        {/* A pane running ssh, split once, with a file dropped into the new split. */}
        <div className="ssh-demo" role="img" aria-label={`An ssh pane split in two: both halves are on devbox in ~/app, and a dropped screenshot is uploaded and its remote path typed. Paste works the same with ${paste}.`}>
          <div className="ssh-pane">
            <div className="ssh-head"><span className="ssh-dot" />ssh devbox <span className="ssh-dir">~/app</span></div>
            <pre className="ssh-body" aria-hidden="true">
              <span className="p">you@devbox</span>:<span className="d">~/app</span>$ git pull{'\n'}
              Already up to date.{'\n'}
              <span className="p">you@devbox</span>:<span className="d">~/app</span>$ <span className="ssh-cursor" />
            </pre>
          </div>
          <div className="ssh-pane ssh-pane-new">
            <div className="ssh-head"><span className="ssh-dot" />ssh devbox <span className="ssh-dir">~/app</span></div>
            <pre className="ssh-body" aria-hidden="true">
              <span className="p">you@devbox</span>:<span className="d">~/app</span>$ claude{'\n'}
              <span className="ssh-typed">/tmp/nex.k3Fa9Q/screenshot.png</span>
            </pre>
            <div className="ssh-file" aria-hidden="true">
              <span className="ssh-file-icon" />screenshot.png
            </div>
            <div className="ssh-toast" aria-hidden="true">Uploaded to devbox</div>
          </div>
        </div>
      </Reveal>

      <div className="ssh-points">
        {POINTS.map((p, i) => (
          <Reveal key={p.title} delay={i * 70} className="ssh-point">
            <h3>{p.title}</h3>
            <p>{p.body}</p>
          </Reveal>
        ))}
      </div>

      <Reveal className="ssh-config">
        <div className="ssh-config-text">
          <h3>Password hosts, asked once</h3>
          <p>
            Uploads and new sessions never type a password. When a host wants one, nex offers to
            add this block to the end of <code>~/.ssh/config</code>. It only matches ssh started
            inside nex, your own settings still win, and deleting it undoes it.
          </p>
          <p className="ssh-off">
            Rather not? <code>ssh-upload = false</code> and <code>ssh-follow = false</code> turn
            each half off.
          </p>
        </div>
        <pre className="ssh-code"><code>{SHARING}</code></pre>
      </Reveal>
    </section>
  )
}
