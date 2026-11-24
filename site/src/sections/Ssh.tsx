import DemoVideo from '../components/DemoVideo'
import Points from '../components/Points'
import Reveal from '../components/Reveal'

/** Copy follows the README's "SSH, built in" and "Agents on ssh hosts" sections, and CHANGELOG.md 0.10.0–0.12.0. */
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
    title: 'Files and Changes on the host',
    body: 'Ctrl+E or Ctrl+G (Cmd on a Mac) in an ssh pane opens the file explorer or the git Changes pane in that session’s folder on the host.',
  },
  {
    title: 'Your mic, on the server',
    body: 'Click a pane header’s mic button and the host’s default microphone becomes yours, so Claude Code’s voice mode works over ssh. Experimental.',
  },
  {
    title: 'One login',
    body: 'Keys and agents just work. For password hosts, nex offers to turn on connection sharing once, and everything after rides on that login.',
  },
  {
    title: 'Plain ssh, one small helper',
    body: 'No wrapper command. nex installs its helper, nex-host, in ~/.nex on the host (no root) so agents outlive the pane. ssh-agents = false skips it.',
  },
]

const SHARING = `# Added by nex: ssh started in a nex pane shares its connection …
Match exec "test x$TERM_PROGRAM = xnex"
  ControlMaster auto
  ControlPath ~/.ssh/nex-%C
  ControlPersist 60`

export default function Ssh() {
  return (
    <section className="section" id="ssh">
      <Reveal className="section-head">
        <p className="eyebrow">SSH</p>
        <h2 className="title">SSH that{' '}<br />follows you.</h2>
        <p className="lede">
          Run plain <code>ssh</code> and the rest of nex comes with you: splits open on the same
          server, Files and Changes work there, and files you drop or paste land there too.
        </p>
      </Reveal>

      <Reveal>
        <DemoVideo
          name="ssh"
          label="ssh localhost, then a split right opens on the same host and folder; Ctrl+E opens the file explorer and Ctrl+G the git Changes pane, both on the host"
          fallback={{
            name: 'hero',
            label: 'Over ssh, new tabs open on the same server: one with the git Changes pane, another with a file explorer, both on the host',
          }}
          width={1200}
          height={750}
        />
      </Reveal>

      <Points items={POINTS} />

      <Reveal className="aside">
        <div>
          <h3>Password hosts, asked once.</h3>
          <p>
            Uploads and new sessions never type a password. When a host wants one, nex offers to
            add this block to the end of <code>~/.ssh/config</code>. It only matches ssh started
            inside nex, your own settings still win, and deleting it undoes it.
          </p>
          <p>
            Rather not? <code>ssh-upload = false</code> and <code>ssh-follow = false</code> turn
            each half off.
          </p>
        </div>
        <pre className="code"><code>{SHARING}</code></pre>
      </Reveal>
    </section>
  )
}
