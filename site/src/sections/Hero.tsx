import DemoVideo from '../components/DemoVideo'
import Icon from '../components/Icon'
import { REPO } from '../links'
import { desktopOf, download, platformIcon, platformName, setPlatform, usePlatform } from '../platform'
import './hero.css'

export default function Hero() {
  const platform = usePlatform()
  const dl = download(platform)
  // Phone visitors are offered the desktop they most likely use.
  const other = desktopOf(platform) === 'mac' ? 'linux' : 'mac'

  return (
    <section className="hero" id="top">
      <a className="hero-badge hero-in" href="#agents">
        New: the Agents pane and pipelines <span aria-hidden="true">›</span>
      </a>
      <h1 className="hero-title hero-in">
        <span className="hero-kicker">nex — GPU terminal emulator with AI agents, SSH, files and git</span>
        The nex terminal.
      </h1>
      <p className="lede hero-sub hero-in">
        A fast, GPU-accelerated terminal for macOS, Linux and Android, written in Rust, with agent
        monitoring, SSH that follows you, a file explorer and git changes built in. No tmux,
        no plugins, no setup.
      </p>
      <div className="actions hero-in">
        <a className="btn btn-primary" href={dl.href}>
          <Icon name="download" size={18} />
          {dl.label}
          <Icon name={platformIcon(platform)} size={18} />
        </a>
        <a className="btn btn-link" href={REPO}>View on GitHub <span aria-hidden="true">›</span></a>
      </div>
      <p className="hero-platforms hero-in">
        <span className="chip"><Icon name="apple" size={16} /> macOS · Apple Silicon &amp; Intel</span>
        <span className="chip"><Icon name="linux" size={16} /> Linux · Wayland &amp; X11</span>
        <a className="chip" href="#mobile"><Icon name="android" size={16} /> Android · iPhone beta</a>
        <button type="button" className="hero-switch" onClick={() => setPlatform(other)}>
          On {platformName(other)}?
        </button>
      </p>
      <div className="hero-stage">
        <DemoVideo
          name="hero"
          label="nex over ssh: a pipeline of agents starts and the Agents pane is pinned beside it; a new tab opens on the same server with the git Changes pane, another with a file explorer, and from the pinned Agents pane the run's team opens and one of its agents is jumped into, its row marked as the one on screen"
          width={1200}
          height={750}
          eager
        />
      </div>
    </section>
  )
}
