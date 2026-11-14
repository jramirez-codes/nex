import { useEffect, useRef } from 'react'
import { REPO, RELEASES, asset } from '../links'
import './hero.css'

export default function Hero() {
  const stage = useRef<HTMLDivElement>(null)

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
      <p className="eyebrow hero-in">Version 0.2 · Linux · Open source</p>
      <h1 className="hero-title hero-in">
        The <span className="gradient-text">nex-gen</span> terminal.
      </h1>
      <p className="lede hero-sub hero-in">
        GPU-rendered and written in Rust. Tabs, splits and a file explorer,
        all built in. No tmux, no plugins, no setup.
      </p>
      <div className="hero-actions hero-in">
        <a className="btn btn-primary" href={RELEASES}>Download for Linux</a>
        <a className="btn btn-ghost" href={REPO}>View on GitHub →</a>
      </div>
      <div className="hero-stage" ref={stage}>
        <div className="hero-glow" aria-hidden="true" />
        <div className="hero-window">
          <img
            className="hero-shot"
            src={asset('demo/tour.gif')}
            alt="nex splitting into git, btop and nvim panes, moving focus between them, and switching tabs"
            width={1200}
            height={750}
          />
        </div>
      </div>
    </section>
  )
}
