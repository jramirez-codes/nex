import { useEffect, useState } from 'react'
import { REPO, RELEASES, asset } from '../links'
import './nav.css'

const LINKS = [
  ['Features', '#features'],
  ['Performance', '#performance'],
  ['Compare', '#compare'],
  ['Install', '#install'],
]

export default function Nav() {
  const [scrolled, setScrolled] = useState(false)

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 8)
    onScroll()
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => window.removeEventListener('scroll', onScroll)
  }, [])

  return (
    <header className={`nav ${scrolled ? 'scrolled' : ''}`}>
      <nav className="nav-inner" aria-label="Main">
        <a className="nav-brand" href="#top">
          <img src={asset('nex.svg')} alt="" width={26} height={26} />
          nex
        </a>
        <div className="nav-links">
          {LINKS.map(([label, href]) => <a key={href} href={href}>{label}</a>)}
          <a href={REPO}>GitHub</a>
        </div>
        <a className="nav-cta" href={RELEASES}>Download</a>
      </nav>
    </header>
  )
}
