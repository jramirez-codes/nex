import { useEffect, useRef, useState } from 'react'
import { asset } from '../links'

/**
 * The nex logo, waking up once (nex-animated.svg plays its ~3.6s intro, then
 * rests on the static pose). With `onView`, the static logo shows until it
 * first scrolls into view and the intro plays then, so a logo far down the
 * page isn't finished before anyone sees it.
 */
export default function AnimatedLogo({ size, onView = false }: { size: number; onView?: boolean }) {
  const ref = useRef<HTMLImageElement>(null)
  const [play, setPlay] = useState(!onView)

  useEffect(() => {
    if (play) return
    const el = ref.current
    if (!el) return
    const io = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) {
        setPlay(true)
        io.disconnect()
      }
    }, { threshold: 0.6 })
    io.observe(el)
    return () => io.disconnect()
  }, [play])

  return (
    <img
      ref={ref}
      src={asset(play ? 'nex-animated.svg' : 'nex.svg')}
      alt=""
      width={size}
      height={size}
    />
  )
}
