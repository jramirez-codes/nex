import Chapter from '../components/Chapter'

/** Copy follows the README's "SSH, built in" section and CHANGELOG.md 0.10.0–0.12.0. */
const POINTS = [
  {
    title: 'Drop it, paste it, done',
    body: 'Files dropped or pasted into an ssh pane are uploaded to the host, and nex types the remote paths.',
  },
  {
    title: 'One login',
    body: 'Keys and agents just work. For password hosts, nex offers connection sharing once, and everything rides on that login.',
  },
  {
    title: 'Plain ssh',
    body: 'No wrapper command. A small helper, nex-host, goes in ~/.nex on the host, no root, so agents outlive the pane.',
  },
]

export default function Ssh() {
  return (
    <Chapter
      id="ssh"
      eyebrow="SSH"
      accent="cyan"
      title={['SSH that', 'follows you.']}
      lede={<>Run plain <code>ssh</code>. Splits open on the same server, in the same folder, and Files and Changes work there too.</>}
      clip={{
        name: 'ssh',
        label: 'ssh localhost, then a split right opens on the same host and folder; Ctrl+E opens the file explorer and Ctrl+G the git Changes pane, both on the host',
      }}
      fallback={{
        name: 'hero',
        label: 'Over ssh, new tabs open on the same server: one with the git Changes pane, another with a file explorer, both on the host',
      }}
      points={POINTS}
    />
  )
}
