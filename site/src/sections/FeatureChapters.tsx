import Chapter from '../components/Chapter'
import { CHAPTERS } from '../data/features'
import { desktopOf, usePlatform } from '../platform'

/** Tabs & splits, Files & Changes and Search: one chapter each, as sibling sections. */
export default function FeatureChapters() {
  const platform = desktopOf(usePlatform())
  return (
    <>
      {CHAPTERS.map(c => (
        <Chapter
          key={c.id}
          id={c.id}
          eyebrow={c.eyebrow}
          accent={c.accent}
          title={c.title}
          lede={c.lede}
          clip={c.clip}
          size={[1000, 625]}
          extra={<p className="chapter-keys">{c.keys[platform].map(k => <kbd key={k}>{k}</kbd>)}</p>}
        />
      ))}
    </>
  )
}
