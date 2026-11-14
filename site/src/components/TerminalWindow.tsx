import type { ReactNode } from 'react'
import './terminal.css'

export interface Tab {
  icon: string
  label: string
}

/** An HTML rendition of nex's window chrome: traffic lights, the segmented tab track with
 * its glass pill, and a body for pane cards. */
export default function TerminalWindow({ tabs, active = 0, children }: {
  tabs: Tab[]
  active?: number
  children: ReactNode
}) {
  return (
    <div className="term" role="img" aria-label="nex window with split panes">
      <div className="term-bar">
        <div className="term-lights"><i /><i /><i /></div>
        <div className="term-tabs">
          {tabs.map((t, i) => (
            <div key={t.label} className={`term-tab ${i === active ? 'active' : ''}`}>
              <span className="term-tab-icon">{t.icon}</span>
              <span className="term-tab-label">{t.label}</span>
            </div>
          ))}
        </div>
        <span className="term-plus">+</span>
      </div>
      <div className="term-body">{children}</div>
    </div>
  )
}

/** One split pane: a rounded card with nex's header (program + directory, split/zoom/close). */
export function Pane({ title, focused = false, className = '', children }: {
  title: string
  focused?: boolean
  className?: string
  children: ReactNode
}) {
  return (
    <section className={`pane ${focused ? 'focused' : ''} ${className}`}>
      <header className="pane-head">
        <span>{title}</span>
        <span className="pane-buttons">◫ ⊟ ⤢ ✕</span>
      </header>
      <div className="pane-body">{children}</div>
    </section>
  )
}
