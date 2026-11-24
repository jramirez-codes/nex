import type { ReactNode } from 'react'
import DemoVideo, { type Clip } from './DemoVideo'
import Points from './Points'
import Reveal from './Reveal'
import './chapter.css'

export type Accent = 'blue' | 'violet' | 'cyan' | 'orange' | 'green'

/**
 * One big feature, the way a product page tells it: a coloured eyebrow, a giant two-line
 * headline, one short lede, then its recording, wide. Optional points sit underneath.
 */
export default function Chapter({ id, eyebrow, accent, title, lede, clip, fallback, size = [1200, 750], extra, points }: {
  id: string
  eyebrow: string
  accent: Accent
  title: [string, string]
  lede: ReactNode
  clip: Clip
  fallback?: Clip
  size?: [number, number]
  /** Shown under the lede, e.g. the chapter's shortcuts. */
  extra?: ReactNode
  points?: { title: string; body: string }[]
}) {
  return (
    <section className="section chapter" id={id} aria-labelledby={`${id}-title`}>
      <Reveal className="chapter-head">
        <p className={`eyebrow accent-${accent}`}>{eyebrow}</p>
        <h2 className="chapter-title" id={`${id}-title`}>{title[0]}{' '}<br />{title[1]}</h2>
        <p className="lede">{lede}</p>
        {extra}
      </Reveal>
      <Reveal className="media">
        <DemoVideo {...clip} fallback={fallback} width={size[0]} height={size[1]} />
      </Reveal>
      {points && <Points items={points} />}
    </section>
  )
}
