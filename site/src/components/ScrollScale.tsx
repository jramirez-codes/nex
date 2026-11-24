import { useEffect, useRef, type ReactNode } from 'react'

/**
 * Grows its children from a little smaller and dimmer to full size as they scroll into view,
 * by setting --p (0 to 1) for CSS. Without JS, or with reduced motion, they simply show.
 */
export default function ScrollScale({ children, className = '' }: { children: ReactNode; className?: string }) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      el.style.setProperty('--p', '1')
      return
    }
    let raf = 0
    const update = () => {
      raf = 0
      const vh = window.innerHeight
      // 0 while its top is at the bottom of the screen, 1 once the top reaches 35% down.
      const p = Math.min(1, Math.max(0, (vh - el.getBoundingClientRect().top) / (vh * 0.65)))
      el.style.setProperty('--p', p.toFixed(3))
    }
    const onScroll = () => { raf ||= requestAnimationFrame(update) }
    update()
    window.addEventListener('scroll', onScroll, { passive: true })
    window.addEventListener('resize', onScroll)
    return () => {
      cancelAnimationFrame(raf)
      window.removeEventListener('scroll', onScroll)
      window.removeEventListener('resize', onScroll)
    }
  }, [])

  return <div ref={ref} className={`scroll-scale ${className}`}>{children}</div>
}
