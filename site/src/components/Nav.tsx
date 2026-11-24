import { useEffect, useState } from 'react'
import { REPO } from '../links'
import { download, platformIcon, usePlatform } from '../platform'
import AnimatedLogo from './AnimatedLogo'
import Icon, { type IconName } from './Icon'
import './nav.css'

/** The site's one header, sticky at the top: the brand, a short list of sections, GitHub and
 *  Download (a burger sheet under 700px). global.css's scroll-padding-top clears its height. */
const LINKS: [string, string, IconName][] = [
  ['Agents', '#agents', 'agents'],
  ['Features', '#features', 'features'],
  ['Install', '#install', 'install'],
  ['FAQ', '#faq', 'faq'],
]

const BREAKPOINT = 700

export default function Nav() {
  const [open, setOpen] = useState(false)
  const platform = usePlatform()
  const dl = download(platform)

  // The mobile menu: close on Escape, and on widening past the breakpoint.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    const wide = window.matchMedia(`(min-width: ${BREAKPOINT + 1}px)`)
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
    <header className={`nav ${open ? 'open' : ''}`}>
      <nav className="nav-inner" aria-label="Main">
        <a className="nav-brand" href="#top" onClick={close}>
          <AnimatedLogo size={24} />
          nex
        </a>
        <div className="nav-links">
          {LINKS.map(([label, href, icon]) => <a key={href} href={href}><Icon name={icon} size={14} />{label}</a>)}
        </div>
        <a className="nav-icon" href={REPO} aria-label="nex on GitHub" title="GitHub">
          <Icon name="github" size={18} />
        </a>
        <a className="nav-cta" href={dl.href}>
          <Icon name="download" size={14} />
          {platform === 'ios' ? 'Build' : 'Download'}
          <Icon name={platformIcon(platform)} size={14} className="nav-cta-os" />
          <span className="sr-only"> for {dl.short}</span>
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
        {LINKS.map(([label, href, icon]) => <a key={href} href={href} onClick={close}><Icon name={icon} size={20} />{label}</a>)}
        <a href={REPO} onClick={close}><Icon name="github" size={20} />GitHub</a>
      </div>
    </header>
  )
}
