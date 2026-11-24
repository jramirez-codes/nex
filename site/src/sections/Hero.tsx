import { useEffect, useRef } from 'react'
import DemoVideo from '../components/DemoVideo'
import { REPO } from '../links'
import { desktopOf, download, platformName, setPlatform, usePlatform } from '../platform'
import './hero.css'

export default function Hero() {
  const stage = useRef<HTMLDivElement>(null)
  const platform = usePlatform()
  // Phone visitors are offered the desktop they most likely use.
  const other = desktopOf(platform) === 'mac' ? 'linux' : 'mac'

  // Tilt the window back a little and let it flatten as the page scrolls.
  useEffect(() => {
    const el = stage.current
    if (!el || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return
    let frame = 0
    const onScroll = () => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        const p = Math.min(1, window.scrollY / 420)
        el.style.setProperty('--tilt', `${(1 - p) * 14}deg`)
        el.style.setProperty('--lift', `${(1 - p) * 30}px`)
      })
    }
    onScroll()
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      cancelAnimationFrame(frame)
      window.removeEventListener('scroll', onScroll)
    }
  }, [])

  return (
    <section className="hero" id="top">
      <a className="hero-badge hero-in" href="#agents">
        <span className="hero-badge-new">New</span>
        Agents pane and pipelines
        <span aria-hidden="true">→</span>
      </a>
      <h1 className="hero-title hero-in">
        <span className="hero-kicker">nex — GPU terminal emulator with AI agents, SSH, files and git</span>
        The <span className="gradient-text">nex</span> terminal.
      </h1>
      <p className="lede hero-sub hero-in">
        A fast, GPU-accelerated terminal for macOS, Linux and Android, written in Rust, with agent
        monitoring, SSH that follows you, a file explorer and git changes built in. No tmux,
        no plugins, no setup.
      </p>
      <div className="hero-actions hero-in">
        <a className="btn btn-primary" href={download(platform).href}>{download(platform).label}</a>
        <a className="btn btn-ghost" href={REPO}>View on GitHub →</a>
      </div>
      <p className="hero-platforms hero-in">
        <span className="chip"><span aria-hidden="true">⌘</span> macOS · Apple Silicon &amp; Intel</span>
        <span className="chip"><span aria-hidden="true">🐧</span> Linux · Wayland &amp; X11</span>
        <a className="chip" href="#mobile"><span aria-hidden="true">📱</span> Android · iPhone beta</a>
        <button type="button" className="hero-switch" onClick={() => setPlatform(other)}>
          On {platformName(other)}?
        </button>
      </p>
      <div className="hero-stage" ref={stage}>
        <div className="hero-glow" aria-hidden="true" />
        <div className="hero-window">
          <DemoVideo
            className="hero-shot"
            name="hero"
            label="nex over ssh: a pipeline of agents starts and the Agents pane is pinned beside it; a new tab opens on the same server with the git Changes pane, another with a file explorer, and from the pinned Agents pane the run's team opens and one of its agents is jumped into, its row marked as the one on screen"
            width={1200}
            height={750}
            eager
          />
        </div>
      </div>
    </section>
  )
}
