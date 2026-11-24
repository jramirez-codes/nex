import { useEffect, useState } from 'react'
import { download, usePlatform } from '../platform'
import './productbar.css'

/** The product's own sticky bar, like Apple's local nav: the name, a few sections, Download. */
const LINKS: [string, string][] = [
  ['Overview', 'top'],
  ['Agents', 'agents'],
  ['SSH', 'ssh'],
  ['Specs', 'specs'],
]

export default function ProductBar() {
  const platform = usePlatform()
  const dl = download(platform)
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState('top')

  // Mark the section crossing the middle of the screen, if it's one of ours.
  useEffect(() => {
    let raf = 0
    const update = () => {
      raf = 0
      const mid = window.innerHeight / 2
      const hit = LINKS.find(([, id]) => {
        const r = document.getElementById(id)?.getBoundingClientRect()
        return r && r.top <= mid && r.bottom > mid
      })
      setActive(hit?.[1] ?? '')
    }
    const onScroll = () => { raf ||= requestAnimationFrame(update) }
    update()
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      cancelAnimationFrame(raf)
      window.removeEventListener('scroll', onScroll)
    }
  }, [])

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open])

  return (
    <div className={`pbar ${open ? 'open' : ''}`}>
      <nav className="pbar-inner" aria-label="nex">
        <a className="pbar-name" href="#top" onClick={() => setOpen(false)}>nex</a>
        <button
          type="button"
          className="pbar-toggle"
          aria-expanded={open}
          aria-controls="pbar-links"
          aria-label={open ? 'Hide sections' : 'Show sections'}
          onClick={() => setOpen(o => !o)}
        >
          <svg className="icon" width="12" height="12" viewBox="0 0 12 12" aria-hidden="true" focusable="false">
            <path d="m2 4.5 4 3.5 4-3.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
        <div className="pbar-links" id="pbar-links">
          {LINKS.map(([label, id]) => (
            <a
              key={id}
              href={`#${id}`}
              aria-current={active === id ? 'location' : undefined}
              onClick={() => setOpen(false)}
            >
              {label}
            </a>
          ))}
        </div>
        <a className="pbar-cta" href={dl.href}>
          {platform === 'ios' ? 'Build' : 'Download'}
          <span className="sr-only"> for {dl.short}</span>
        </a>
      </nav>
    </div>
  )
}
