import { useEffect, useRef } from 'react'
import DemoVideo from '../components/DemoVideo'
import { REPO, RELEASES } from '../links'
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
      <p className="eyebrow hero-in">Open-source terminal emulator for Linux</p>
      <h1 className="hero-title hero-in">
        The <span className="gradient-text">nex</span> terminal.
      </h1>
      <p className="lede hero-sub hero-in">
        A fast, GPU-accelerated terminal for Linux, written in Rust. Tabs, split
        panes and a file explorer, all built in. No tmux, no plugins, no setup.
      </p>
      <div className="hero-actions hero-in">
        <a className="btn btn-primary" href={RELEASES}>Download for Linux</a>
        <a className="btn btn-ghost" href={REPO}>View on GitHub →</a>
      </div>
      <div className="hero-stage" ref={stage}>
        <div className="hero-glow" aria-hidden="true" />
        <div className="hero-window">
          <DemoVideo
            className="hero-shot"
            name="tour"
            label="nex splitting into git, btop and nvim panes, moving focus between them, and switching tabs"
            width={1200}
            height={750}
            eager
          />
        </div>
      </div>
    </section>
  )
}
