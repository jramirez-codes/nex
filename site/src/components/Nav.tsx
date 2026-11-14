import { useEffect, useState } from 'react'
import { REPO, RELEASES, asset } from '../links'
import { platformName, usePlatform } from '../platform'
import './nav.css'

const LINKS = [
  ['macOS', '#mac'],
  ['Features', '#features'],
  ['Shortcuts', '#shortcuts'],
  ['Performance', '#performance'],
  ['Compare', '#compare'],
  ['Install', '#install'],
  ['FAQ', '#faq'],
]

export default function Nav() {
  const [scrolled, setScrolled] = useState(false)
  const [open, setOpen] = useState(false)
  const platform = usePlatform()

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 8)
    onScroll()
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  // The mobile menu: close on Escape, and on widening past the breakpoint.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    const wide = window.matchMedia('(min-width: 861px)')
    const onWide = () => wide.matches && setOpen(false)
    window.addEventListener('keydown', onKey)
    wide.addEventListener('change', onWide)
    return () => {
      window.removeEventListener('keydown', onKey)
      wide.removeEventListener('change', onWide)
    }
  }, [open])

  const close = () => setOpen(false)

  return (
    <header className={`nav ${scrolled || open ? 'scrolled' : ''} ${open ? 'open' : ''}`}>
      <nav className="nav-inner" aria-label="Main">
        <a className="nav-brand" href="#top" onClick={close}>
          <img src={asset('nex.svg')} alt="" width={26} height={26} />
          nex
        </a>
        <div className="nav-links">
          {LINKS.map(([label, href]) => <a key={href} href={href}>{label}</a>)}
          <a href={REPO}>GitHub</a>
        </div>
        <a className="nav-cta" href={RELEASES}>
          Download<span className="nav-cta-os"> for {platformName(platform)}</span>
        </a>
        <button
          type="button"
          className="nav-burger"
          aria-expanded={open}
          aria-controls="nav-sheet"
          aria-label={open ? 'Close menu' : 'Open menu'}
          onClick={() => setOpen(o => !o)}
        >
          <span /><span />
        </button>
      </nav>
      <div className="nav-sheet" id="nav-sheet" hidden={!open}>
        {LINKS.map(([label, href]) => <a key={href} href={href} onClick={close}>{label}</a>)}
        <a href={REPO} onClick={close}>GitHub</a>
      </div>
    </header>
  )
}
