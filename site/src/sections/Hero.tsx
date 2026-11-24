import DemoVideo from '../components/DemoVideo'
import Icon from '../components/Icon'
import ScrollScale from '../components/ScrollScale'
import { download, platformIcon, usePlatform } from '../platform'
import './hero.css'

/** Copy follows the README's headline: a terminal with tabs, splits, files, git, SSH and agents built in. */
export default function Hero() {
  const platform = usePlatform()
  const dl = download(platform)

  return (
    <section className="hero" id="top">
      <div className="hero-copy">
        <h1 className="hero-title hero-in">
          <span className="hero-kicker">nex — GPU terminal emulator with AI agents, SSH, files and git</span>
          <span className="hero-name">nex</span>
        </h1>
        <p className="hero-tagline hero-in">The terminal, with your agents built in.</p>
        <div className="actions hero-in">
          <a className="btn btn-primary" href={dl.href}>
            <Icon name="download" size={18} />
            {dl.label}
            <Icon name={platformIcon(platform)} size={18} />
          </a>
          <a className="btn btn-link" href="#agents">Learn more <span aria-hidden="true">›</span></a>
        </div>
        <p className="hero-platforms hero-in">
          <span className="chip"><Icon name="apple" size={15} /> macOS</span>
          <span className="chip"><Icon name="linux" size={15} /> Linux</span>
          <span className="chip"><Icon name="android" size={15} /> Android</span>
          <span className="chip">iPhone beta</span>
        </p>
      </div>
      <ScrollScale className="hero-stage">
        <div className="media">
          <DemoVideo
            name="hero"
            label="nex over ssh: a pipeline of agents starts and the Agents pane is pinned beside it; a new tab opens on the same server with the git Changes pane, another with a file explorer, and from the pinned Agents pane the run's team opens and one of its agents is jumped into, its row marked as the one on screen"
            width={1200}
            height={750}
            eager
          />
        </div>
      </ScrollScale>
    </section>
  )
}
